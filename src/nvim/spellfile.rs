#![forbid(unsafe_code)]

//! Code for reading and writing spell files (`src/nvim/spellfile.c`).

use std::ffi::CStr;

use crate::api::private::helpers::cstr_as_obj;
use crate::arglist::get_arglist_exp;
use crate::buffer::{FileBuf, buflist_findname_exp};
use crate::charset::{NUL, ascii_isdigit, getdigits_int, skipdigits_slice};
use crate::cmdexpand::Cmdidx;
use crate::cursor::Linenr;
use crate::drawscreen::{UPD_SOME_VALID, redraw_all_later};
use crate::errors::*;
use crate::ex_docmd::Exarg;
use crate::fileio::{
    EOF, SEEK_SET, buf_reload, get2c, get3c, get4c, get8ctime, put_bytes, put_time, read_string,
    vim_fgets, vim_tempname,
};
use crate::globals::*;
use crate::mbyte::{
    Vimconv, convert_setup, enc_canonize, mb_charlen, mb_cptr2char_adv, mb_ptr2char_adv,
    string_convert, utf_char2bytes, utf_ptr2char, utf_slice2char, utf_valid_string, utfc_ptr2len,
};
use crate::memline::{ml_append_buf, ml_get_buf};
use crate::message::{
    emsg, msg, msg_clr_eos, msg_outtrans_long, msg_puts, msg_start, semsg, smsg, verbose_enter,
    verbose_leave,
};
use crate::option::{K_OPT_SPELLFILE, OPT_LOCAL, copy_option_part, set_option_value_give_err};
use crate::option_vars::*;
use crate::os::env::home_replace;
use crate::os::fs::{CFile, MAXPATHL, os_isdir, os_mkdir, os_mkdir_recurse, os_path_exists};
use crate::os::input::{fast_breakcheck, line_breakcheck, veryfast_breakcheck};
use crate::os::stdpaths::{XDGVarType, get_xdg_home};
use crate::os::time::{Timestamp, os_time, time_t};
use crate::path::{
    K_PATH_CMP_EXPAND, K_PATH_CMP_LITERAL, dir_of_file_exists, path_equal, path_tail,
    path_tail_with_sep, vim_ispathsep,
};
use crate::regexp::{RE_MAGIC, RE_STRICT, RE_STRING, Regprog, vim_regexec_prog};
use crate::runtime::{ETYPE_SPELL, estack_pop, estack_push};
use crate::spell::{
    E_FORMAT, MAXWLEN, SP_FORMERROR, WF_BANNED, WF_COMPROOT, WF_FIXCAP, WF_HAS_AFF, WF_KEEPCAP,
    WF_NEEDCOMP, WF_NOCOMPAFT, WF_NOCOMPBEF, WF_RARE, WF_REGION, WFP_NC, captype,
    clear_spell_chartab, close_spellbuf, count_common_word, init_spell_chartab, init_syl_tab,
    onecap_copy, open_spellbuf, parse_spelllang, slang_alloc, slang_clear, slang_clear_sug,
    spell_casefold, spell_enc, spell_soundfold, spell_toupper, with_spell_ext, with_spell_priv,
};
use crate::spellsuggest::{Fromto, Idx, Salitem, Slang, SlangId, Spelltab};
use crate::strings::{format_to_buf, gettext, strerror, vim_strchr};
use crate::syntax::win_s;
use crate::ui::ui_flush;
use crate::undo::buf_is_changed_fn;
use std::mem::size_of;

pub type SpellAddType = i32;
use crate::change::TAB;
use crate::viml::parser::parser::CONV_NONE;

pub const SPELL_ADD_GOOD: SpellAddType = 0;
pub const SPELL_ADD_BAD: SpellAddType = 1;
pub const SPELL_ADD_RARE: SpellAddType = 2;

pub(crate) const MAXREGIONS: usize = 8;

const SPL_FNAME_TMPL: &std::ffi::CStr = c"%s.%s.spl";
const SPL_FNAME_ADD: &[u8] = b".add.";
const SPL_FNAME_ASCII: &[u8] = b".ascii.";

const VIMSUGMAGIC: &[u8; 6] = b"VIMsug";
const VIMSUGMAGICL: usize = 6;
const VIMSUGVERSION: i32 = 1;

const WF_AFX: i32 = 0x20;

pub(crate) const WF_NOSUGGEST: i32 = 0x0400;

const WFP_UP: i32 = 0x04;
const WFP_COMPPERMIT: i32 = 0x08;
const WFP_COMPFORBID: i32 = 0x10;

const COMP_CHECKDUP: i32 = 1;
const COMP_CHECKREP: i32 = 2;
const COMP_CHECKCASE: i32 = 4;
const COMP_CHECKTRIPLE: i32 = 8;

const SP_TRUNCERROR: i32 = -1;
const SP_OTHERERROR: i32 = -3;

const BY_NOFLAGS: i32 = 0;
const BY_INDEX: i32 = 1;
const BY_FLAGS: i32 = 2;
const BY_FLAGS2: i32 = 3;
const BY_SPECIAL: i32 = BY_FLAGS2;

const ZERO_FLAG: u32 = 65009;

const SAL_F0LLOWUP: i32 = 1;
const SAL_COLLAPSE: i32 = 2;
const SAL_REM_ACCENTS: i32 = 4;

const VIMSPELLMAGIC: &[u8; 8] = b"VIMspell";
const VIMSPELLMAGICL: usize = 8;
const VIMSPELLVERSION: i32 = 50;

const SN_REGION: i32 = 0;
const SN_CHARFLAGS: i32 = 1;
const SN_MIDWORD: i32 = 2;
const SN_PREFCOND: i32 = 3;
const SN_REP: i32 = 4;
const SN_SAL: i32 = 5;
const SN_SOFO: i32 = 6;
const SN_MAP: i32 = 7;
const SN_COMPOUND: i32 = 8;
const SN_SYLLABLE: i32 = 9;
const SN_NOBREAK: i32 = 10;
const SN_SUGFILE: i32 = 11;
const SN_REPSAL: i32 = 12;
const SN_WORDS: i32 = 13;
const SN_NOSPLITSUGS: i32 = 14;
const SN_INFO: i32 = 15;
const SN_NOCOMPOUNDSUGS: i32 = 16;
const SN_END: i32 = 255;

const SNF_REQUIRED: i32 = 1;

const CF_WORD: u8 = 0x01;
const CF_UPPER: u8 = 0x02;

const COMPOUND_MAX_LEN: usize = 100000;

const MAXLINELEN: usize = 500;
const MAXITEMCNT: usize = 30;

const AFT_CHAR: i32 = 0;
const AFT_LONG: i32 = 1;
const AFT_CAPLONG: i32 = 2;
const AFT_NUM: i32 = 3;

const AH_KEY_LEN: usize = 17;
const SBLOCKSIZE: usize = 16000;
const WN_MASK: i32 = 0xffff;
const SHARED_MASK: Idx = 0x0800_0000;

const PFX_FLAGS: i32 = -256;

const CONDIT_COMB: i32 = 1;
const CONDIT_CFIX: i32 = 2;
const CONDIT_SUF: i32 = 4;
const CONDIT_AFF: i32 = 8;

const E_SPELL_TRUNC: &std::ffi::CStr = c"E758: Truncated spell file";
const E_ERROR_WHILE_READING_SUG_FILE_STR: &std::ffi::CStr =
    c"E782: Error while reading .sug file: %s";
const E_DUPLICATE_CHAR_IN_MAP_ENTRY: &std::ffi::CStr = c"E783: Duplicate char in MAP entry";
const E_ILLEGAL_CHARACTER_IN_WORD: &std::ffi::CStr = c"E1280: Illegal character in word";
const E_AFFTRAILING: &std::ffi::CStr = c"Trailing text in %s line %d: %s";
const E_AFFNAME: &std::ffi::CStr = c"Affix name too long in %s line %d: %s";
const MSG_COMPRESSING: &std::ffi::CStr = c"Compressing word tree...";

struct SpellfileModuleState {
    compress_start: std::cell::Cell<i32>,
    compress_inc: std::cell::Cell<i32>,
    compress_added: std::cell::Cell<i32>,
}

impl SpellfileModuleState {
    const fn new() -> Self {
        Self {
            compress_start: std::cell::Cell::new(30000),
            compress_inc: std::cell::Cell::new(100),
            compress_added: std::cell::Cell::new(500000),
        }
    }
}

std::thread_local! {
    static SPELLFILE_STATE: SpellfileModuleState = const { SpellfileModuleState::new() };
}

#[inline]
fn spellfile_st<R>(f: impl FnOnce(&SpellfileModuleState) -> R) -> R {
    SPELLFILE_STATE.with(f)
}

#[repr(C)]
struct Afffile {
    af_enc: Option<std::ffi::CString>,
    af_flagtype: i32,
    af_rare: u32,
    af_keepcase: u32,
    af_bad: u32,
    af_needaffix: u32,
    af_circumfix: u32,
    af_needcomp: u32,
    af_comproot: u32,
    af_compforbid: u32,
    af_comppermit: u32,
    af_nosuggest: u32,
    af_pfxpostpone: i32,
    af_ignoreextra: bool,
    af_pref: std::collections::BTreeMap<Vec<u8>, Affheader>,
    af_suff: std::collections::BTreeMap<Vec<u8>, Affheader>,
    af_comp: std::collections::BTreeMap<Vec<u8>, Compitem>,
}

#[repr(C)]
struct Affentry {
    ae_chop: Option<std::ffi::CString>,
    ae_add: Option<std::ffi::CString>,
    ae_flags: Option<std::ffi::CString>,
    ae_cond: Option<std::ffi::CString>,
    ae_prog: Option<Box<Regprog>>,
    ae_compforbid: u8,
    ae_comppermit: u8,
}

impl Affentry {
    #[inline]
    fn chop_cstr(&self) -> Option<&std::ffi::CStr> {
        self.ae_chop.as_deref()
    }

    #[inline]
    fn add_cstr(&self) -> Option<&std::ffi::CStr> {
        self.ae_add.as_deref()
    }

    #[inline]
    fn flags_cstr(&self) -> Option<&std::ffi::CStr> {
        self.ae_flags.as_deref()
    }

    #[inline]
    fn cond_cstr(&self) -> Option<&std::ffi::CStr> {
        self.ae_cond.as_deref()
    }
}

#[repr(C)]
struct Affheader {
    ah_key: [u8; AH_KEY_LEN],
    ah_flag: u32,
    ah_new_id: i32,
    ah_combine: i32,
    ah_follows: i32,
    ah_entries: Vec<Affentry>,
}

#[repr(C)]
struct Compitem {
    ci_key: [u8; AH_KEY_LEN],
    ci_flag: u32,
    ci_new_id: i32,
}

#[derive(Clone, Copy)]
enum NodeSlot {
    Sibling(usize),
    Child(usize),
}

impl NodeSlot {
    fn set(self, nodes: &mut [Wordnode], val: Option<usize>) {
        match self {
            Self::Sibling(idx) => nodes[idx].wn_sibling = val,
            Self::Child(idx) => nodes[idx].wn_child = val,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Wordnode {
    wn_index: i32,
    wn_wnode: Option<usize>,
    wn_child: Option<usize>,
    wn_sibling: Option<usize>,
    wn_refs: i32,
    wn_byte: u8,
    wn_affix_id: u8,
    wn_flags: u16,
    wn_region: i16,
}

impl Wordnode {
    const fn new() -> Self {
        Self {
            wn_index: 0,
            wn_wnode: None,
            wn_child: None,
            wn_sibling: None,
            wn_refs: 0,
            wn_byte: 0,
            wn_affix_id: 0,
            wn_flags: 0,
            wn_region: 0,
        }
    }
}

struct Spellinfo {
    si_foldroot: Option<usize>,
    si_foldwcount: i32,
    si_keeproot: Option<usize>,
    si_keepwcount: i32,
    si_prefroot: Option<usize>,
    si_sugtree: i32,
    si_nodes: Vec<Wordnode>,
    si_blocks_cnt: i32,
    si_compress_cnt: i32,
    si_first_free: Option<usize>,
    si_free_count: i32,
    si_spellbuf: Option<std::rc::Rc<FileBuf>>,
    si_ascii: bool,
    si_add: i32,
    si_clear_chartab: i32,
    si_region: i32,
    si_conv: Vimconv,
    si_memtot: i32,
    si_verbose: bool,
    si_msg_count: i32,
    si_info: Option<std::ffi::CString>,
    si_region_count: i32,
    si_region_name: [u8; MAXREGIONS * 2 + 1],
    si_rep: Vec<Fromto>,
    si_repsal: Vec<Fromto>,
    si_sal: Vec<Fromto>,
    si_sofofr: Option<std::ffi::CString>,
    si_sofoto: Option<std::ffi::CString>,
    si_nosugfile: i32,
    si_nosplitsugs: i32,
    si_nocompoundsugs: i32,
    si_followup: bool,
    si_collapse: bool,
    si_commonwords: std::collections::BTreeSet<Vec<u8>>,
    si_sugtime: time_t,
    si_rem_accents: bool,
    si_map: Vec<u8>,
    si_midword: Option<std::ffi::CString>,
    si_compmax: i32,
    si_compminlen: i32,
    si_compsylmax: i32,
    si_compoptions: i32,
    si_comppat: Vec<std::ffi::CString>,
    si_compflags: Option<std::ffi::CString>,
    si_nobreak: u8,
    si_syllable: Option<std::ffi::CString>,
    si_prefcond: Vec<Option<std::ffi::CString>>,
    si_newpref_id: i32,
    si_newcomp_id: i32,
}

impl Default for Spellinfo {
    fn default() -> Self {
        Self {
            si_foldroot: None,
            si_foldwcount: 0,
            si_keeproot: None,
            si_keepwcount: 0,
            si_prefroot: None,
            si_sugtree: 0,
            si_nodes: Vec::new(),
            si_blocks_cnt: 0,
            si_compress_cnt: 0,
            si_first_free: None,
            si_free_count: 0,
            si_spellbuf: None,
            si_ascii: false,
            si_add: 0,
            si_clear_chartab: 0,
            si_region: 0,
            si_conv: Vimconv::new(),
            si_memtot: 0,
            si_verbose: false,
            si_msg_count: 0,
            si_info: None,
            si_region_count: 0,
            si_region_name: [0; MAXREGIONS * 2 + 1],
            si_rep: Vec::new(),
            si_repsal: Vec::new(),
            si_sal: Vec::new(),
            si_sofofr: None,
            si_sofoto: None,
            si_nosugfile: 0,
            si_nosplitsugs: 0,
            si_nocompoundsugs: 0,
            si_followup: false,
            si_collapse: false,
            si_commonwords: std::collections::BTreeSet::new(),
            si_sugtime: 0,
            si_rem_accents: false,
            si_map: Vec::new(),
            si_midword: None,
            si_compmax: 0,
            si_compminlen: 0,
            si_compsylmax: 0,
            si_compoptions: 0,
            si_comppat: Vec::new(),
            si_compflags: None,
            si_nobreak: 0,
            si_syllable: None,
            si_prefcond: Vec::new(),
            si_newpref_id: 0,
            si_newcomp_id: 0,
        }
    }
}

#[inline]
fn spell_getc(fd: &CFile) -> i32 {
    fd.getc()
}

#[inline]
fn spell_putc(c: i32, fd: &CFile) -> i32 {
    fd.putc(c)
}

#[inline]
fn spell_write(bytes: &[u8], fd: &CFile) -> usize {
    usize::from(fd.write_all(bytes))
}

#[inline]
fn spell_close(fd: CFile) -> i32 {
    fd.close()
}

#[inline]
fn spell_read_bytes(buf: &mut [u8], fd: &CFile) -> i32 {
    let read_bytes = fd.read_bytes(buf);
    if read_bytes != buf.len() {
        if fd.is_eof() {
            SP_TRUNCERROR
        } else {
            SP_OTHERERROR
        }
    } else {
        0
    }
}

#[inline]
fn spell_read_nonnul_bytes(buf: &mut [u8], fd: &CFile) -> i32 {
    let ret = spell_read_bytes(buf, fd);
    if ret != 0 {
        return ret;
    }
    if buf.contains(&0) {
        return SP_FORMERROR;
    }
    0
}

#[inline]
fn spell_check_magic_string(fd: &CFile) -> i32 {
    let mut buf = [0u8; VIMSPELLMAGICL];
    let ret = spell_read_bytes(&mut buf, fd);
    if ret != 0 {
        return ret;
    }
    if buf[..VIMSPELLMAGICL] != VIMSPELLMAGIC[..VIMSPELLMAGICL] {
        return SP_FORMERROR;
    }
    0
}

/// Load one spell file and store the info into a Slang.
pub fn spell_load_file(
    fname_cstr: &std::ffi::CStr,
    mut lang: Option<&mut [u8]>,
    silent: bool,
) -> Option<SlangId> {
    let lang_cstr = lang
        .as_deref()
        .map(|b| crate::memory::bytes_to_cstring(crate::memory::bytes_until_nul(b)));
    let mut new_lp = slang_alloc(lang_cstr.as_deref());
    new_lp.sl_fname = Some(fname_cstr.to_owned());
    new_lp.sl_add = path_tail(fname_cstr)
        .to_bytes()
        .windows(SPL_FNAME_ADD.len())
        .any(|w| w == SPL_FNAME_ADD);

    if !spell_load_file_into(fname_cstr, &mut new_lp, silent) {
        if let Some(ref mut lang) = lang
            && !lang.is_empty()
        {
            lang[0] = 0;
        }
        return None;
    }

    if lang.is_some() {
        let id = with_spell_ext(|ext| {
            let mut fl = ext.first_lang.borrow_mut();
            let idx = fl.len();
            fl.push(Some(std::rc::Rc::from(new_lp)));
            SlangId::from_slot(idx)
        });
        Some(id)
    } else {
        None
    }
}

fn spell_load_file_into(fname_cstr: &std::ffi::CStr, lp: &mut Slang, silent: bool) -> bool {
    let Some(fd) = CFile::open(fname_cstr, c"r") else {
        if !silent {
            semsg!(gettext(E_NOTOPEN), fname_cstr);
        } else if opt_vars().p_verbose.get() > 2 {
            verbose_enter();
            smsg!(0, E_NOTOPEN, fname_cstr);
            verbose_leave();
        }
        return false;
    };
    if opt_vars().p_verbose.get() > 2 {
        verbose_enter();
        smsg!(0, gettext(c"Reading spell file \"%s\""), fname_cstr);
        verbose_leave();
    }

    estack_push(ETYPE_SPELL, Some(fname_cstr), 0);
    let did_estack_push = true;

    let mut fail = false;
    let scms_ret = spell_check_magic_string(&fd);
    match scms_ret {
        SP_FORMERROR | SP_TRUNCERROR => {
            semsg!(
                c"%s",
                gettext(c"E757: This does not look like a spell file"),
            );
            fail = true;
        }
        SP_OTHERERROR => {
            semsg!(
                gettext(c"E5042: Failed to read spell file %s: %s"),
                fname_cstr,
                strerror(fd.ferror()),
            );
            fail = true;
        }
        _ => {}
    }

    if !fail {
        let c = spell_getc(&fd);
        if c < VIMSPELLVERSION {
            emsg(gettext(c"E771: Old spell file, needs to be updated"));
            fail = true;
        } else if c > VIMSPELLVERSION {
            emsg(gettext(c"E772: Spell file is for newer version of Vim"));
            fail = true;
        }
    }

    let mut res: i32 = 0;
    let mut check_someerror = false;
    let mut had_soundfold = false;
    while !fail {
        let n = spell_getc(&fd);
        if n == SN_END {
            break;
        }
        let c = spell_getc(&fd);
        let mut len = get4c(&fd);
        if len < 0 {
            res = SP_TRUNCERROR;
            check_someerror = true;
            break;
        }

        res = 0;
        match n {
            SN_INFO => {
                let Some(raw) = read_string(&fd, len as usize) else {
                    lp.sl_info = None;
                    fail = true;
                    break;
                };
                lp.sl_info = Some(crate::memory::cstring_until_nul(&raw));
            }
            SN_REGION => {
                res = read_region_section(&fd, &mut *lp, len);
            }
            SN_CHARFLAGS => {
                res = read_charflags_section(&fd);
            }
            SN_MIDWORD => {
                let Some(raw) = read_string(&fd, len as usize) else {
                    lp.sl_midword = None;
                    fail = true;
                    break;
                };
                lp.sl_midword = Some(crate::memory::cstring_until_nul(&raw));
            }
            SN_PREFCOND => {
                res = read_prefcond_section(&fd, &mut *lp);
            }
            SN_REP => {
                res = read_rep_section(&fd, &mut lp.sl_rep, &mut lp.sl_rep_first);
            }
            SN_REPSAL => {
                res = read_rep_section(&fd, &mut lp.sl_repsal, &mut lp.sl_repsal_first);
            }
            SN_SAL | SN_SOFO => {
                // there should either be a SN_SAL or SN_SOFO section
                if had_soundfold {
                    res = SP_FORMERROR;
                } else if n == SN_SAL {
                    res = read_sal_section(&fd, &mut *lp);
                } else {
                    res = read_sofo_section(&fd, &mut *lp);
                }
                had_soundfold = true;
            }
            SN_MAP => {
                let Some(p) = read_string(&fd, len as usize) else {
                    fail = true;
                    break;
                };
                set_map_str(&mut *lp, &p);
            }
            SN_WORDS => {
                res = read_words_section(&fd, &mut *lp, len);
            }
            SN_SUGFILE => {
                lp.sl_sugtime = get8ctime(&fd);
            }
            SN_NOSPLITSUGS => {
                lp.sl_nosplitsugs = true;
            }
            SN_NOCOMPOUNDSUGS => {
                lp.sl_nocompoundsugs = true;
            }
            SN_COMPOUND => {
                res = read_compound(&fd, &mut *lp, len);
            }
            SN_NOBREAK => {
                lp.sl_nobreak.set(true);
            }
            SN_SYLLABLE => {
                let Some(raw) = read_string(&fd, len as usize) else {
                    lp.sl_syllable = None;
                    fail = true;
                    break;
                };
                lp.sl_syllable = Some(crate::memory::cstring_until_nul(&raw));
                if !init_syl_tab(&mut *lp) {
                    fail = true;
                    break;
                }
            }
            _ => {
                if (c & SNF_REQUIRED) != 0 {
                    emsg(gettext(c"E770: Unsupported section in spell file"));
                    fail = true;
                    break;
                }
                while len > 0 {
                    len -= 1;
                    if spell_getc(&fd) < 0 {
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
        res = spell_read_tree(&fd, &mut lp.sl_fbyts, &mut lp.sl_fidxs, false, 0);
        if res != 0 {
            check_someerror = true;
        }
    }

    if !fail && !check_someerror {
        res = spell_read_tree(&fd, &mut lp.sl_kbyts, &mut lp.sl_kidxs, false, 0);
        if res != 0 {
            check_someerror = true;
        }
    }

    if !fail && !check_someerror {
        res = spell_read_tree(
            &fd,
            &mut lp.sl_pbyts,
            &mut lp.sl_pidxs,
            true,
            lp.sl_prefprog.len() as i32,
        );
        if res != 0 {
            check_someerror = true;
        }
    }

    if check_someerror {
        if res == SP_FORMERROR {
            emsg(gettext(E_FORMAT));
            fail = true;
        } else if res == SP_TRUNCERROR {
            emsg(gettext(E_SPELL_TRUNC));
            fail = true;
        } else if res == SP_OTHERERROR {
            fail = true;
        }
    }

    spell_close(fd);
    if did_estack_push {
        estack_pop();
    }

    !fail
}

fn tree_count_words(byts: &[u8], idxs: &mut [Idx]) {
    let byts_len = byts.len() as i32;
    let mut arridx = [0 as Idx; MAXWLEN];
    let mut curi = [0_i32; MAXWLEN];
    let mut wordcount = [0_i32; MAXWLEN];

    arridx[0] = 0;
    curi[0] = 1;
    wordcount[0] = 0;
    let mut depth: isize = 0;
    while depth >= 0 && !g_st().got_int.get() {
        let d = depth as usize;
        if curi[d] > i32::from(byts[arridx[d] as usize]) {
            idxs[arridx[d] as usize] = wordcount[d];
            if depth > 0 {
                wordcount[d - 1] += wordcount[d];
            }
            depth -= 1;
            fast_breakcheck();
        } else {
            let mut n = arridx[d] + curi[d];
            curi[d] += 1;

            let c = i32::from(byts[n as usize]);
            if c == 0 {
                wordcount[d] += 1;
                while n + 1 < byts_len && byts[(n + 1) as usize] == 0 {
                    n += 1;
                    curi[d] += 1;
                }
            } else if d < MAXWLEN - 1 {
                depth += 1;
                let nd = depth as usize;
                arridx[nd] = idxs[n as usize];
                curi[nd] = 1;
                wordcount[nd] = 0;
            }
        }
    }
}

/// Load the .sug files for languages that have one and weren't loaded yet.
pub fn suggest_load_files() {
    let mut buf = [0u8; MAXWLEN];
    let mut ga: Vec<u8> = Vec::new();

    let mut lpi: usize = 0;
    while lpi < win_s(&curwin_mut()).b_langp.borrow().len() {
        let lp_slang_id = win_s(&curwin_mut()).b_langp.borrow()[lpi].lp_slang;
        let idx = (lp_slang_id.0.unwrap().get() - 1) as usize;
        let need_load = lp_slang_id.rc().sl_sugtime != 0 && !lp_slang_id.rc().sl_sugloaded;
        if need_load {
            let rc = with_spell_ext(|ext| ext.first_lang.borrow_mut()[idx].take().unwrap());
            let mut slang_box =
                Box::new(std::rc::Rc::try_unwrap(rc).unwrap_or_else(|_| unreachable!()));
            let slang = &mut *slang_box;
            slang.sl_sugloaded = true;

            let Some(ref fname_cstr) = slang.sl_fname else {
                with_spell_ext(|ext| {
                    ext.first_lang.borrow_mut()[idx] = Some(std::rc::Rc::from(slang_box))
                });
                lpi += 1;
                continue;
            };
            let Some(prefix_len) = fname_cstr.to_bytes().iter().rposition(|&b| b == b'.') else {
                with_spell_ext(|ext| {
                    ext.first_lang.borrow_mut()[idx] = Some(std::rc::Rc::from(slang_box))
                });
                lpi += 1;
                continue;
            };
            if !path_equal(
                &fname_cstr.as_c_str()[prefix_len..],
                c".spl",
                K_PATH_CMP_LITERAL,
            ) {
                with_spell_ext(|ext| {
                    ext.first_lang.borrow_mut()[idx] = Some(std::rc::Rc::from(slang_box))
                });
                lpi += 1;
                continue;
            }
            let mut sug_bytes = fname_cstr.as_bytes()[..prefix_len].to_vec();
            sug_bytes.extend_from_slice(b".sug");
            let sug_fname = std::ffi::CString::new(sug_bytes).unwrap();
            if let Some(fd) = CFile::open(&sug_fname, c"r") {
                let mut ok = true;
                for slot in buf.iter_mut().take(VIMSUGMAGICL) {
                    *slot = spell_getc(&fd) as u8;
                }
                if buf[..VIMSUGMAGICL] != VIMSUGMAGIC[..VIMSUGMAGICL] {
                    semsg!(
                        gettext(c"E778: This does not look like a .sug file: %s"),
                        &sug_fname,
                    );
                    ok = false;
                }
                if ok {
                    let c = spell_getc(&fd);
                    if c < VIMSUGVERSION {
                        semsg!(
                            gettext(c"E779: Old .sug file, needs to be updated: %s"),
                            &sug_fname,
                        );
                        ok = false;
                    } else if c > VIMSUGVERSION {
                        semsg!(
                            gettext(c"E780: .sug file is for newer version of Vim: %s",),
                            &sug_fname,
                        );
                        ok = false;
                    }
                }

                if ok {
                    let timestamp = get8ctime(&fd);
                    if timestamp != slang.sl_sugtime {
                        semsg!(
                            gettext(c"E781: .sug file doesn't match .spl file: %s"),
                            &sug_fname,
                        );
                        ok = false;
                    }
                }

                if ok {
                    let mut someerror = false;
                    if spell_read_tree(&fd, &mut slang.sl_sbyts, &mut slang.sl_sidxs, false, 0) != 0
                    {
                        someerror = true;
                    }

                    if !someerror {
                        *slang.sl_sugbuf.borrow_mut() = Some(open_spellbuf());
                        let sugbuf_borrow = slang.sl_sugbuf.borrow();
                        let sugbuf = sugbuf_borrow.as_deref().unwrap();
                        let wcount = get4c(&fd);
                        if wcount < 0 {
                            someerror = true;
                        } else {
                            ga.reserve(100);
                            for wordnr in 0..wcount {
                                ga.clear();
                                loop {
                                    let c = spell_getc(&fd);
                                    if c < 0 {
                                        someerror = true;
                                        break;
                                    }
                                    ga.push(c as u8);
                                    if c == NUL as i32 {
                                        break;
                                    }
                                }
                                if someerror {
                                    break;
                                }
                                let text_len = ga.len().saturating_sub(1);
                                if !ml_append_buf(
                                    sugbuf,
                                    wordnr as Linenr,
                                    &ga[..text_len],
                                    ga.len() as i32,
                                    true,
                                ) {
                                    someerror = true;
                                    break;
                                }
                            }
                            ga.clear();
                        }
                    }

                    if someerror {
                        semsg!(gettext(E_ERROR_WHILE_READING_SUG_FILE_STR), &sug_fname);
                        slang_clear_sug(&mut *slang);
                    } else {
                        tree_count_words(&slang.sl_fbyts, &mut slang.sl_fidxs);
                        tree_count_words(&slang.sl_sbyts, &mut slang.sl_sidxs);
                    }
                }
                spell_close(fd);
            }
            with_spell_ext(|ext| {
                ext.first_lang.borrow_mut()[idx] = Some(std::rc::Rc::from(slang_box))
            });
        }
        lpi += 1;
    }
}

fn read_cnt_string(fd: &CFile, cnt_bytes: i32, cntp: &mut i32) -> Option<Vec<u8>> {
    let mut cnt: i32 = 0;
    for _ in 0..cnt_bytes {
        let c = spell_getc(fd);
        if c == EOF {
            *cntp = SP_TRUNCERROR;
            return None;
        }
        cnt = ((cnt as u32) << 8).wrapping_add(c as u32) as i32;
    }
    *cntp = cnt;
    if cnt == 0 {
        return None;
    }
    let str_ = read_string(fd, cnt as usize);
    if str_.is_none() {
        *cntp = SP_OTHERERROR;
    }
    str_
}

fn read_region_section(fd: &CFile, lp: &mut Slang, len: i32) -> i32 {
    if len > (MAXREGIONS * 2) as i32 {
        return SP_FORMERROR;
    }
    let ret = spell_read_nonnul_bytes(&mut lp.sl_regions[..len as usize], fd);
    if ret != 0 {
        return ret;
    }
    lp.sl_regions[len as usize] = 0;
    0
}

fn read_charflags_section(fd: &CFile) -> i32 {
    let mut flagslen: i32 = 0;
    let mut follen: i32 = 0;

    let flags = read_cnt_string(fd, 1, &mut flagslen);
    if flagslen < 0 {
        return flagslen;
    }

    let fol = read_cnt_string(fd, 2, &mut follen);
    if follen < 0 {
        return follen;
    }

    if let (Some(flags), Some(fol)) = (&flags, &fol) {
        set_spell_charflags(flags, flagslen, fol);
    }

    if flags.is_none() != fol.is_none() {
        return SP_FORMERROR;
    }
    0
}

fn read_prefcond_section(fd: &CFile, lp: &mut Slang) -> i32 {
    let cnt = get2c(fd);
    if cnt <= 0 {
        return SP_FORMERROR;
    }

    lp.sl_prefprog.resize_with(cnt as usize, || None);

    for i in 0..cnt {
        let n = spell_getc(fd);
        if n < 0 || n >= MAXWLEN as i32 {
            return SP_FORMERROR;
        }

        if n > 0 {
            let mut buf = [0u8; MAXWLEN + 1];
            buf[0] = b'^';
            let ret = spell_read_nonnul_bytes(&mut buf[1..1 + n as usize], fd);
            if ret != 0 {
                return ret;
            }
            buf[(n + 1) as usize] = 0;
            lp.sl_prefprog.as_mut_slice()[i as usize] = crate::regexp::vim_regcomp_box(
                crate::memory::bytes_as_cstr(&buf),
                RE_MAGIC | RE_STRING,
            );
        }
    }
    0
}

fn read_rep_section(fd: &CFile, gap: &mut Vec<Fromto>, first: &mut [i16; 256]) -> i32 {
    let cnt = get2c(fd);
    if cnt < 0 {
        return SP_TRUNCERROR;
    }

    gap.reserve(cnt as usize);

    while (gap.len() as i32) < cnt {
        let mut c: i32 = 0;
        let raw_from = read_cnt_string(fd, 1, &mut c);
        if c < 0 {
            return c;
        }
        let Some(raw_from) = raw_from else {
            return SP_FORMERROR;
        };
        let ft_from = crate::memory::cstring_until_nul(&raw_from);
        let raw_to = read_cnt_string(fd, 1, &mut c);
        if c <= 0 {
            if c < 0 {
                return c;
            }
            return SP_FORMERROR;
        }
        let ft_to = raw_to.map_or_else(std::ffi::CString::default, |v| {
            crate::memory::cstring_until_nul(&v)
        });
        gap.push(Fromto { ft_from, ft_to });
    }

    first.fill(-1);
    for (i, ftp) in gap.iter().enumerate() {
        let b = ftp.ft_from.as_bytes()[0] as usize;
        if first[b] == -1 {
            first[b] = i as i16;
        }
    }
    0
}

fn read_sal_section(fd: &CFile, slang: &mut Slang) -> i32 {
    slang.sl_sofo = false;

    let flags = spell_getc(fd);
    if (flags & SAL_F0LLOWUP) != 0 {
        slang.sl_followup = true;
    }
    if (flags & SAL_COLLAPSE) != 0 {
        slang.sl_collapse = true;
    }
    if (flags & SAL_REM_ACCENTS) != 0 {
        slang.sl_rem_accents = true;
    }

    let cnt = get2c(fd);
    if cnt < 0 {
        return SP_TRUNCERROR;
    }

    let gap = &mut slang.sl_sal;
    gap.clear();
    gap.reserve((cnt + 1) as usize);

    while (gap.len() as i32) < cnt {
        let mut c: i32 = NUL as i32;
        let mut ccnt = spell_getc(fd);
        if ccnt < 0 {
            return SP_TRUNCERROR;
        }
        let mut buf = vec![0u8; ccnt as usize + 2];
        let mut pos: usize = 0;

        let mut i: i32 = 0;
        while i < ccnt {
            c = spell_getc(fd);
            if !vim_strchr(c"0123456789(-<^$", c)
                .map_or(std::ptr::null_mut(), |s| s.as_ptr().cast_mut())
                .is_null()
            {
                break;
            }
            buf[pos] = c as u8;
            pos += 1;
            i += 1;
        }
        buf[pos] = 0;
        pos += 1;

        let sm_oneof_off = if c == b'(' as i32 {
            let oneof_off = pos;
            i += 1;
            while i < ccnt {
                c = spell_getc(fd);
                if c == b')' as i32 {
                    break;
                }
                buf[pos] = c as u8;
                pos += 1;
                i += 1;
            }
            buf[pos] = 0;
            pos += 1;
            i += 1;
            if i < ccnt {
                c = spell_getc(fd);
            }
            Some(oneof_off)
        } else {
            None
        };

        let sm_rules_off = pos;
        if i < ccnt {
            buf[pos] = c as u8;
            pos += 1;
        }
        i += 1;
        if i < ccnt {
            let rem = (ccnt - i) as usize;
            let ret = spell_read_nonnul_bytes(&mut buf[pos..pos + rem], fd);
            if ret != 0 {
                return ret;
            }
            pos += rem;
        }
        buf[pos] = 0;

        let sm_to = read_cnt_string(fd, 1, &mut ccnt);
        if ccnt < 0 {
            return ccnt;
        }

        let buf_bytes = crate::memory::bytes_until_nul(&buf);
        let sm_lead_w = mb_str2wide(buf_bytes);
        let sm_leadlen = mb_charlen(buf_bytes);
        let sm_oneof_w = if let Some(off) = sm_oneof_off {
            mb_str2wide(crate::memory::bytes_until_nul(&buf[off..]))
        } else {
            Vec::new()
        };
        let sm_to_w = match sm_to {
            Some(ref s) => mb_str2wide(crate::memory::bytes_until_nul(s)),
            None => Vec::new(),
        };
        let sm_rules = crate::memory::cstring_until_nul(&buf[sm_rules_off..]);
        gap.push(Salitem {
            sm_leadlen,
            sm_rules,
            sm_lead_w,
            sm_oneof_w,
            sm_to_w,
        });
    }

    if !gap.is_empty() {
        gap.push(Salitem {
            sm_leadlen: 0,
            sm_rules: std::ffi::CString::default(),
            sm_lead_w: mb_str2wide(b""),
            sm_oneof_w: Vec::new(),
            sm_to_w: Vec::new(),
        });
    }

    set_sal_first(slang);
    0
}

fn read_words_section(fd: &CFile, lp: &mut Slang, len: i32) -> i32 {
    let mut done: i32 = 0;
    let mut word = [0u8; MAXWLEN];

    while done < len {
        let mut i: usize = 0;
        loop {
            let c = spell_getc(fd);
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

        count_common_word(lp, &word, -1, 10);
        done += (i + 1) as i32;
    }
    0
}

fn read_sofo_section(fd: &CFile, slang: &mut Slang) -> i32 {
    let mut cnt: i32 = 0;
    let from = read_cnt_string(fd, 2, &mut cnt);
    if cnt < 0 {
        return cnt;
    }

    let to = read_cnt_string(fd, 2, &mut cnt);
    if cnt < 0 {
        return cnt;
    }

    match (from, to) {
        (Some(from), Some(to)) => set_sofo(slang, &from, &to),
        (None, None) => 0,
        _ => SP_FORMERROR,
    }
}

fn read_compound(fd: &CFile, slang: &mut Slang, len: i32) -> i32 {
    let mut todo = len;
    let mut cnt: i32 = 0;

    if todo < 2 {
        return SP_FORMERROR;
    }
    todo -= 1;
    let mut c = spell_getc(fd);
    if c < 2 {
        c = MAXWLEN as i32;
    }
    slang.sl_compmax = c;

    todo -= 1;
    c = spell_getc(fd);
    if c < 1 {
        c = 0;
    }
    slang.sl_compminlen = c;

    todo -= 1;
    c = spell_getc(fd);
    if c < 1 {
        c = MAXWLEN as i32;
    }
    slang.sl_compsylmax = c;

    c = spell_getc(fd);
    if c != 0 {
        fd.ungetc(c);
    } else {
        todo -= 1;
        c = spell_getc(fd);
        todo -= 1;
        slang.sl_compoptions = c;

        let gap = &mut slang.sl_comppat;
        c = get2c(fd);
        if c < 0 {
            return SP_TRUNCERROR;
        }
        todo -= 2;
        gap.clear();
        gap.reserve(c as usize);
        while c > 0 {
            c -= 1;
            let raw = read_cnt_string(fd, 1, &mut cnt);
            if cnt < 0 {
                return cnt;
            }
            gap.push(raw.map_or_else(std::ffi::CString::default, |v| {
                crate::memory::cstring_until_nul(&v)
            }));
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
    let mut pat_vec = vec![0u8; patsize];

    let mut cp_vec = Vec::with_capacity(flagsize);
    let mut ap_vec = Vec::with_capacity(flagsize);
    let mut crp_vec: Option<Vec<u8>> = Some(Vec::with_capacity(flagsize));

    pat_vec[..3].copy_from_slice(b"^\\(");
    let mut pos: usize = 3;

    let mut atstart: i32 = 1;
    while todo > 0 {
        todo -= 1;
        c = spell_getc(fd);
        if c == EOF {
            return SP_TRUNCERROR;
        }

        if vim_strchr(c"?*+[]/", c)
            .map_or(std::ptr::null_mut(), |s| s.as_ptr().cast_mut())
            .is_null()
            && !ap_vec.contains(&(c as u8))
        {
            ap_vec.push(c as u8);
        }

        if atstart != 0 {
            if c == b'[' as i32 {
                atstart = 2;
            } else if c == b']' as i32 {
                atstart = 0;
            } else {
                if !cp_vec.contains(&(c as u8)) {
                    cp_vec.push(c as u8);
                }
                if atstart == 1 {
                    atstart = 0;
                }
            }
        }

        if let Some(ref mut crp) = crp_vec {
            if c == b'?' as i32 || c == b'+' as i32 || c == b'*' as i32 {
                crp_vec = None;
            } else {
                crp.push(c as u8);
            }
        }

        if c == b'/' as i32 {
            pat_vec[pos] = b'\\';
            pat_vec[pos + 1] = b'|';
            pos += 2;
            atstart = 1;
        } else {
            if c == b'?' as i32 || c == b'+' as i32 || c == b'~' as i32 {
                pat_vec[pos] = b'\\';
                pos += 1;
            }
            pos += utf_char2bytes(c, &mut pat_vec[pos..]) as usize;
        }
    }

    pat_vec[pos..pos + 4].copy_from_slice(b"\\)$\0");

    slang.sl_compstartflags = Some(std::ffi::CString::new(cp_vec).unwrap_or_default());
    slang.sl_compallflags = Some(std::ffi::CString::new(ap_vec).unwrap_or_default());
    slang.sl_comprules = crp_vec.map(|v| std::ffi::CString::new(v).unwrap_or_default());

    slang.sl_compprog = crate::regexp::vim_regcomp_box(
        crate::memory::bytes_as_cstr(&pat_vec),
        RE_MAGIC + RE_STRING + RE_STRICT,
    );
    drop(pat_vec);
    if slang.sl_compprog.is_none() {
        return SP_FORMERROR;
    }

    0
}

fn set_sofo(lp: &mut Slang, from: &[u8], to: &[u8]) -> i32 {
    lp.sl_sal = Vec::new();
    lp.sl_sofo_sal = Vec::new();

    let gap = &mut lp.sl_sofo_sal;
    gap.resize_with(256, Vec::new);
    lp.sl_sofo = true;
    lp.sl_sal_first = [0; 256];

    let mut p = crate::memory::bytes_until_nul(from);
    let mut s = crate::memory::bytes_until_nul(to);
    while !p.is_empty() && !s.is_empty() {
        let c = mb_cptr2char_adv(&mut p);
        let i = mb_cptr2char_adv(&mut s);
        if c >= 256 {
            gap[(c & 0xff) as usize].push((c, i));
        } else {
            lp.sl_sal_first[c as usize] = i;
        }
    }
    if !p.is_empty() || !s.is_empty() {
        return SP_FORMERROR;
    }

    0
}

fn set_sal_first(lp: &mut Slang) {
    let gap = &mut lp.sl_sal;
    lp.sl_sal_first.fill(-1);
    let len = gap.len() as i32;
    let mut i: i32 = 0;
    while i < len {
        let c = (gap[i as usize].sm_lead_w[0] & 0xff) as usize;
        if lp.sl_sal_first[c] == -1 {
            lp.sl_sal_first[c] = i;

            while i + 1 < len && (gap[(i + 1) as usize].sm_lead_w[0] & 0xff) as usize == c {
                i += 1;
            }

            let mut n: i32 = 1;
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

fn mb_str2wide(mut rem: &[u8]) -> Vec<i32> {
    let mut res = Vec::with_capacity(mb_charlen(rem) as usize + 1);
    while !rem.is_empty() {
        res.push(mb_ptr2char_adv(&mut rem));
    }
    res.push(0);
    res
}

fn spell_read_tree(
    fd: &CFile,
    bytsp: &mut Vec<u8>,
    idxsp: &mut Vec<Idx>,
    prefixtree: bool,
    prefixcnt: i32,
) -> i32 {
    let len = get4c(fd);
    if len < 0 {
        return SP_TRUNCERROR;
    }
    if (len as usize) > usize::MAX / size_of::<i32>() {
        return SP_FORMERROR;
    }
    if len <= 0 {
        return 0;
    }

    bytsp.clear();
    bytsp.resize(len as usize, 0);
    idxsp.clear();
    idxsp.resize(len as usize, 0);

    let idx = read_tree_node(fd, bytsp, idxsp, len, 0, prefixtree, prefixcnt, 0);
    if idx < 0 {
        return idx;
    }
    if idx != len {
        return SP_FORMERROR;
    }
    0
}

fn read_tree_node(
    fd: &CFile,
    byts: &mut [u8],
    idxs: &mut [Idx],
    maxidx: i32,
    startidx: Idx,
    prefixtree: bool,
    maxprefcondnr: i32,
    depth: i32,
) -> Idx {
    let mut idx = startidx;

    if depth > MAXWLEN as i32 {
        return SP_FORMERROR;
    }

    let len = spell_getc(fd);
    if len <= 0 {
        return SP_TRUNCERROR;
    }

    if len >= maxidx - startidx {
        return SP_FORMERROR;
    }
    byts[idx as usize] = len as u8;
    idx += 1;

    for _ in 1..=len {
        let mut c = spell_getc(fd);
        if c < 0 {
            return SP_TRUNCERROR;
        }
        if c <= BY_SPECIAL {
            if c == BY_NOFLAGS && !prefixtree {
                idxs[idx as usize] = 0;
            } else if c != BY_INDEX {
                if prefixtree {
                    if c == BY_FLAGS {
                        let n = spell_getc(fd);
                        if n < 0 {
                            return SP_TRUNCERROR;
                        }
                        c = n << 24;
                    } else {
                        c = 0;
                    }

                    let mut n = spell_getc(fd);
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
                    c = spell_getc(fd);
                    if c < 0 {
                        return SP_TRUNCERROR;
                    }
                    if c2 == BY_FLAGS2 {
                        let n = spell_getc(fd);
                        if n < 0 {
                            return SP_TRUNCERROR;
                        }
                        c += n << 8;
                    }
                    if (c & WF_REGION) != 0 {
                        let n = spell_getc(fd);
                        if n < 0 {
                            return SP_TRUNCERROR;
                        }
                        c += n << 16;
                    }
                    if (c & WF_AFX) != 0 {
                        let n = spell_getc(fd);
                        if n < 0 {
                            return SP_TRUNCERROR;
                        }
                        c = (((n as u32) << 24) as i32).wrapping_add(c);
                    }
                }

                idxs[idx as usize] = c;
                c = 0;
            } else {
                let n = get3c(fd);
                if n < 0 || n >= maxidx {
                    return SP_FORMERROR;
                }
                idxs[idx as usize] = n + SHARED_MASK;
                c = spell_getc(fd);
            }
        }
        byts[idx as usize] = c as u8;
        idx += 1;
    }

    for i in 1..=len {
        let pos = (startidx + i) as usize;
        if byts[pos] != 0 {
            if (idxs[pos] & SHARED_MASK) != 0 {
                idxs[pos] &= !SHARED_MASK;
            } else {
                idxs[pos] = idx;
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

fn spell_reload_one(fname_cstr: &std::ffi::CStr, added_word: bool) {
    let mut didit = false;

    let len = with_spell_ext(|ext| ext.first_lang.borrow().len());
    for i in (0..len).rev() {
        let matches = with_spell_ext(|ext| {
            ext.first_lang.borrow()[i].as_ref().is_some_and(|slang| {
                slang
                    .sl_fname
                    .as_ref()
                    .is_some_and(|f| path_equal(fname_cstr, f, K_PATH_CMP_EXPAND))
            })
        });
        if matches {
            let rc = with_spell_ext(|ext| ext.first_lang.borrow_mut()[i].take().unwrap());
            let mut slang_box =
                Box::new(std::rc::Rc::try_unwrap(rc).unwrap_or_else(|_| unreachable!()));
            slang_clear(&mut slang_box);
            if !spell_load_file_into(fname_cstr, &mut slang_box, false) {
                slang_clear(&mut slang_box);
            }
            with_spell_ext(|ext| {
                ext.first_lang.borrow_mut()[i] = Some(std::rc::Rc::from(slang_box));
            });
            redraw_all_later(UPD_SOME_VALID);
            didit = true;
        }
    }

    if added_word && !didit {
        parse_spelllang(&curwin_mut());
    }
}

/// Check the 'mkspellmem' option. Return FAIL if it's wrong.
pub fn spell_check_msm(value: &[u8], apply: bool) -> bool {
    let mut rem = crate::memory::bytes_until_nul(value);

    if !rem.first().is_some_and(u8::is_ascii_digit) {
        return false;
    }
    let start = (getdigits_int(&mut rem, true, 0) * 10) / ((SBLOCKSIZE / 102) as i32);
    let Some(after_comma) = rem.strip_prefix(b",") else {
        return false;
    };
    rem = after_comma;
    if !rem.first().is_some_and(u8::is_ascii_digit) {
        return false;
    }
    let incr = (getdigits_int(&mut rem, true, 0) * 102) / ((SBLOCKSIZE / 10) as i32);
    let Some(after_comma) = rem.strip_prefix(b",") else {
        return false;
    };
    rem = after_comma;
    if !rem.first().is_some_and(u8::is_ascii_digit) {
        return false;
    }
    let added = getdigits_int(&mut rem, true, 0) * 1024;
    if !rem.is_empty() {
        return false;
    }

    if start == 0 || incr == 0 || added == 0 || incr > start {
        return false;
    }

    if apply {
        spellfile_st(|st| {
            st.compress_start.set(start);
            st.compress_inc.set(incr);
            st.compress_added.set(added);
        });
    }
    true
}

fn spell_read_aff(spin: &mut Spellinfo, fname_cstr: &std::ffi::CStr) -> Option<Afffile> {
    let fname = fname_cstr;
    let mut rline = [0u8; MAXLINELEN];
    let mut pc_vec: Vec<u8>;
    let mut lnum: i32 = 0;
    let mut cur_aff_key: Option<(bool, Vec<u8>)> = None;
    let mut did_postpone_prefix = false;
    let mut aff_todo: i32 = 0;
    let mut has_low = false;
    let mut has_fol = false;
    let mut has_upp = false;
    let mut found_map = false;
    let mut compminlen: i32 = 0;
    let mut compsylmax: i32 = 0;
    let mut compoptions: i32 = 0;
    let mut compmax: i32 = 0;
    let mut compflags: Option<std::ffi::CString> = None;
    let mut midword: Option<std::ffi::CString> = None;
    let mut syllable: Option<std::ffi::CString> = None;
    let mut sofofrom: Option<std::ffi::CString> = None;
    let mut sofoto: Option<std::ffi::CString> = None;

    let Some(fd) = CFile::open(fname, c"r") else {
        semsg!(gettext(E_NOTOPEN), fname);
        return None;
    };

    format_to_buf!(
        &mut *g_st().iobuff_bytes_mut(),
        gettext(c"Reading affix file %s..."),
        fname,
    );
    spell_message(&*spin, &g_st().iobuff_cstr());

    let do_rep = spin.si_rep.is_empty();
    let do_repsal = spin.si_repsal.is_empty();
    let do_sal = spin.si_sal.is_empty();
    let do_mapline = spin.si_map.is_empty();

    let mut aff = Afffile {
        af_enc: None,
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

    while !vim_fgets(&mut rline, &fd) && !g_st().got_int.get() {
        line_breakcheck();
        lnum += 1;

        if rline[0] == b'#' {
            continue;
        }

        let mut items: [&CStr; MAXITEMCNT] = [c""; MAXITEMCNT];
        let line_buf: &mut [u8] = if spin.si_conv.vc_type != CONV_NONE {
            let Some(pc_bytes) =
                string_convert(&spin.si_conv, crate::memory::bytes_until_nul(&rline), None)
            else {
                smsg!(
                    0,
                    gettext(c"Conversion failure for word in %s line %d: %s"),
                    fname,
                    lnum,
                    crate::memory::bytes_as_cstr(&rline),
                );
                continue;
            };
            pc_vec = pc_bytes;
            &mut pc_vec
        } else {
            &mut rline
        };

        let mut itemcnt: usize = 0;
        let mut item_ranges = [(0usize, 0usize); MAXITEMCNT];
        let mut p_idx = 0usize;
        let line_end = crate::memory::bytes_until_nul(line_buf).len();
        loop {
            let p_bytes = &line_buf[p_idx..line_end];
            let skip_ws = p_bytes.iter().take_while(|&&b| b <= b' ').count();
            p_idx += skip_ws;
            let p_bytes = &line_buf[p_idx..line_end];
            if p_bytes.is_empty() || itemcnt == MAXITEMCNT {
                break;
            }
            let item_start = p_idx;
            itemcnt += 1;
            let item0_bytes = if itemcnt == 2 {
                &line_buf[item_ranges[0].0..item_ranges[0].1]
            } else {
                &[]
            };
            let item_len = if itemcnt == 2 && spell_info_item(item0_bytes) {
                p_bytes
                    .iter()
                    .take_while(|&&b| b >= b' ' || b == TAB)
                    .count()
            } else {
                p_bytes.iter().take_while(|&&b| b > b' ').count()
            };
            p_idx += item_len;
            item_ranges[itemcnt - 1] = (item_start, p_idx);
            if p_idx == line_end {
                break;
            }
            line_buf[p_idx] = 0;
            p_idx += 1;
        }
        for i in 0..itemcnt {
            let (start, end) = item_ranges[i];
            items[i] = crate::memory::bytes_as_cstr(&line_buf[start..=end]);
        }

        if itemcnt > 0 {
            let itemcnt_i = itemcnt as i32;
            let item0_bytes = items[0].to_bytes();
            if is_aff_rule(&items, itemcnt_i, c"SET", 2) && aff.af_enc.is_none() {
                aff.af_enc = Some(enc_canonize(items[1]));
                let af_enc_opt = aff.af_enc.as_deref();
                if !spin.si_ascii
                    && !convert_setup(
                        &mut spin.si_conv,
                        af_enc_opt,
                        Some(opt_vars().p_enc.as_cstr()),
                    )
                {
                    smsg!(
                        0,
                        gettext(c"Conversion in %s not supported: from %s to %s"),
                        fname,
                        aff.af_enc.as_deref().unwrap_or(c""),
                        &opt_vars().p_enc,
                    );
                }
                spin.si_conv.vc_fail = true;
            } else if is_aff_rule(&items, itemcnt_i, c"FLAG", 2) && aff.af_flagtype == AFT_CHAR {
                match items[1].to_bytes() {
                    b"long" => aff.af_flagtype = AFT_LONG,
                    b"num" => aff.af_flagtype = AFT_NUM,
                    b"caplong" => aff.af_flagtype = AFT_CAPLONG,
                    _ => {
                        smsg!(
                            0,
                            gettext(c"Invalid value for FLAG in %s line %d: %s"),
                            fname,
                            lnum,
                            items[1],
                        );
                    }
                }
                if aff.af_rare != 0
                    || aff.af_keepcase != 0
                    || aff.af_bad != 0
                    || aff.af_needaffix != 0
                    || aff.af_circumfix != 0
                    || aff.af_needcomp != 0
                    || aff.af_comproot != 0
                    || aff.af_nosuggest != 0
                    || compflags.is_some()
                    || !aff.af_suff.is_empty()
                    || !aff.af_pref.is_empty()
                {
                    smsg!(
                        0,
                        gettext(c"FLAG after using flags in %s line %d: %s"),
                        fname,
                        lnum,
                        items[1],
                    );
                }
            } else if spell_info_item(item0_bytes) && itemcnt > 1 {
                let mut bytes = spin
                    .si_info
                    .as_ref()
                    .map_or_else(Vec::new, |s| s.to_bytes().to_vec());
                if !bytes.is_empty() {
                    bytes.push(b'\n');
                }
                bytes.extend_from_slice(item0_bytes);
                bytes.push(b' ');
                bytes.extend_from_slice(items[1].to_bytes());
                spin.si_info = Some(std::ffi::CString::new(bytes).unwrap());
            } else if is_aff_rule(&items, itemcnt_i, c"MIDWORD", 2) && midword.is_none() {
                midword = Some(items[1].to_owned());
            } else if is_aff_rule(&items, itemcnt_i, c"TRY", 2) {
                // ignored
            } else if (is_aff_rule(&items, itemcnt_i, c"RAR", 2)
                || is_aff_rule(&items, itemcnt_i, c"RARE", 2))
                && aff.af_rare == 0
            {
                aff.af_rare = affitem2flag(aff.af_flagtype, items[1], fname_cstr, lnum);
            } else if (is_aff_rule(&items, itemcnt_i, c"KEP", 2)
                || is_aff_rule(&items, itemcnt_i, c"KEEPCASE", 2))
                && aff.af_keepcase == 0
            {
                aff.af_keepcase = affitem2flag(aff.af_flagtype, items[1], fname_cstr, lnum);
            } else if (is_aff_rule(&items, itemcnt_i, c"BAD", 2)
                || is_aff_rule(&items, itemcnt_i, c"FORBIDDENWORD", 2))
                && aff.af_bad == 0
            {
                aff.af_bad = affitem2flag(aff.af_flagtype, items[1], fname_cstr, lnum);
            } else if is_aff_rule(&items, itemcnt_i, c"NEEDAFFIX", 2) && aff.af_needaffix == 0 {
                aff.af_needaffix = affitem2flag(aff.af_flagtype, items[1], fname_cstr, lnum);
            } else if is_aff_rule(&items, itemcnt_i, c"CIRCUMFIX", 2) && aff.af_circumfix == 0 {
                aff.af_circumfix = affitem2flag(aff.af_flagtype, items[1], fname_cstr, lnum);
            } else if is_aff_rule(&items, itemcnt_i, c"NOSUGGEST", 2) && aff.af_nosuggest == 0 {
                aff.af_nosuggest = affitem2flag(aff.af_flagtype, items[1], fname_cstr, lnum);
            } else if (is_aff_rule(&items, itemcnt_i, c"NEEDCOMPOUND", 2)
                || is_aff_rule(&items, itemcnt_i, c"ONLYINCOMPOUND", 2))
                && aff.af_needcomp == 0
            {
                aff.af_needcomp = affitem2flag(aff.af_flagtype, items[1], fname_cstr, lnum);
            } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDROOT", 2) && aff.af_comproot == 0 {
                aff.af_comproot = affitem2flag(aff.af_flagtype, items[1], fname_cstr, lnum);
            } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDFORBIDFLAG", 2)
                && aff.af_compforbid == 0
            {
                aff.af_compforbid = affitem2flag(aff.af_flagtype, items[1], fname_cstr, lnum);
                if !aff.af_pref.is_empty() {
                    smsg!(
                        0,
                        gettext(
                            c"Defining COMPOUNDFORBIDFLAG after PFX item may give wrong results in %s line %d",
                        ),
                        fname,
                        lnum,
                    );
                }
            } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDPERMITFLAG", 2)
                && aff.af_comppermit == 0
            {
                aff.af_comppermit = affitem2flag(aff.af_flagtype, items[1], fname_cstr, lnum);
                if !aff.af_pref.is_empty() {
                    smsg!(
                        0,
                        gettext(
                            c"Defining COMPOUNDPERMITFLAG after PFX item may give wrong results in %s line %d",
                        ),
                        fname,
                        lnum,
                    );
                }
            } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDFLAG", 2) && compflags.is_none() {
                let mut bytes = items[1].to_bytes().to_vec();
                bytes.push(b'+');
                compflags = Some(std::ffi::CString::new(bytes).unwrap());
            } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDRULES", 2) {
                if crate::charset::parse_i64_prefix(items[1].to_bytes()) as i32 == 0 {
                    smsg!(
                        0,
                        gettext(c"Wrong COMPOUNDRULES value in %s line %d: %s"),
                        fname,
                        lnum,
                        items[1],
                    );
                }
            } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDRULE", 2) {
                if compflags.is_some() || !skipdigits_slice(items[1].to_bytes()).is_empty() {
                    let mut bytes = compflags
                        .as_ref()
                        .map_or_else(Vec::new, |s| s.to_bytes().to_vec());
                    if !bytes.is_empty() {
                        bytes.push(b'/');
                    }
                    bytes.extend_from_slice(items[1].to_bytes());
                    compflags = Some(std::ffi::CString::new(bytes).unwrap());
                }
            } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDWORDMAX", 2) && compmax == 0 {
                compmax = crate::charset::parse_i64_prefix(items[1].to_bytes()) as i32;
                if compmax == 0 {
                    smsg!(
                        0,
                        gettext(c"Wrong COMPOUNDWORDMAX value in %s line %d: %s"),
                        fname,
                        lnum,
                        items[1],
                    );
                }
            } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDMIN", 2) && compminlen == 0 {
                compminlen = crate::charset::parse_i64_prefix(items[1].to_bytes()) as i32;
                if compminlen == 0 {
                    smsg!(
                        0,
                        gettext(c"Wrong COMPOUNDMIN value in %s line %d: %s"),
                        fname,
                        lnum,
                        items[1],
                    );
                }
            } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDSYLMAX", 2) && compsylmax == 0 {
                compsylmax = crate::charset::parse_i64_prefix(items[1].to_bytes()) as i32;
                if compsylmax == 0 {
                    smsg!(
                        0,
                        gettext(c"Wrong COMPOUNDSYLMAX value in %s line %d: %s"),
                        fname,
                        lnum,
                        items[1],
                    );
                }
            } else if is_aff_rule(&items, itemcnt_i, c"CHECKCOMPOUNDDUP", 1) {
                compoptions |= COMP_CHECKDUP;
            } else if is_aff_rule(&items, itemcnt_i, c"CHECKCOMPOUNDREP", 1) {
                compoptions |= COMP_CHECKREP;
            } else if is_aff_rule(&items, itemcnt_i, c"CHECKCOMPOUNDCASE", 1) {
                compoptions |= COMP_CHECKCASE;
            } else if is_aff_rule(&items, itemcnt_i, c"CHECKCOMPOUNDTRIPLE", 1) {
                compoptions |= COMP_CHECKTRIPLE;
            } else if is_aff_rule(&items, itemcnt_i, c"CHECKCOMPOUNDPATTERN", 2) {
                if crate::charset::parse_i64_prefix(items[1].to_bytes()) as i32 == 0 {
                    smsg!(
                        0,
                        gettext(c"Wrong CHECKCOMPOUNDPATTERN value in %s line %d: %s"),
                        fname,
                        lnum,
                        items[1],
                    );
                }
            } else if is_aff_rule(&items, itemcnt_i, c"CHECKCOMPOUNDPATTERN", 3) {
                let gap = &mut spin.si_comppat;
                let item1 = items[1];
                let item2 = items[2];
                let mut i: usize = 0;
                while i + 1 < gap.len() {
                    if gap[i].as_c_str() == item1 && gap[i + 1].as_c_str() == item2 {
                        break;
                    }
                    i += 2;
                }
                if i + 1 >= gap.len() {
                    gap.push(item1.to_owned());
                    gap.push(item2.to_owned());
                }
            } else if is_aff_rule(&items, itemcnt_i, c"SYLLABLE", 2) && syllable.is_none() {
                syllable = Some(items[1].to_owned());
            } else if is_aff_rule(&items, itemcnt_i, c"NOBREAK", 1) {
                spin.si_nobreak = 1;
            } else if is_aff_rule(&items, itemcnt_i, c"NOSPLITSUGS", 1) {
                spin.si_nosplitsugs = 1;
            } else if is_aff_rule(&items, itemcnt_i, c"NOCOMPOUNDSUGS", 1) {
                spin.si_nocompoundsugs = 1;
            } else if is_aff_rule(&items, itemcnt_i, c"NOSUGFILE", 1) {
                spin.si_nosugfile = 1;
            } else if is_aff_rule(&items, itemcnt_i, c"PFXPOSTPONE", 1) {
                aff.af_pfxpostpone = 1;
            } else if is_aff_rule(&items, itemcnt_i, c"IGNOREEXTRA", 1) {
                aff.af_ignoreextra = true;
            } else if (item0_bytes == b"PFX" || item0_bytes == b"SFX")
                && aff_todo == 0
                && itemcnt >= 4
            {
                let mut lasti: usize = 4;
                let mut key = [0u8; AH_KEY_LEN];

                let is_pref = item0_bytes[0] == b'P';
                let tp = if is_pref {
                    &mut aff.af_pref
                } else {
                    &mut aff.af_suff
                };

                let item2_bytes = items[2].to_bytes();
                crate::memory::copy_bytes_to_c_buf(&mut key, items[1].to_bytes());
                let key_bytes = crate::memory::bytes_until_nul(&key);
                let cur_aff = if let Some(existing_aff) = tp.get_mut(key_bytes) {
                    if existing_aff.ah_combine != i32::from(item2_bytes.first() == Some(&b'Y')) {
                        smsg!(
                            0,
                            gettext(
                                c"Different combining flag in continued affix block in %s line %d: %s",
                            ),
                            fname,
                            lnum,
                            items[1],
                        );
                    }
                    if existing_aff.ah_follows == 0 {
                        smsg!(
                            0,
                            gettext(c"Duplicate affix in %s line %d: %s"),
                            fname,
                            lnum,
                            items[1],
                        );
                    }
                    existing_aff
                } else {
                    let ah_flag = affitem2flag(aff.af_flagtype, items[1], fname_cstr, lnum);
                    if ah_flag == 0 || items[1].to_bytes().len() >= AH_KEY_LEN {
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
                                c"Affix also used for BAD/RARE/KEEPCASE/NEEDAFFIX/NEEDCOMPOUND/NOSUGGEST in %s line %d: %s",
                            ),
                            fname,
                            lnum,
                            items[1],
                        );
                    }
                    let mut new_ah = Affheader {
                        ah_key: [0; AH_KEY_LEN],
                        ah_flag,
                        ah_new_id: 0,
                        ah_combine: i32::from(item2_bytes.first() == Some(&b'Y')),
                        ah_follows: 0,
                        ah_entries: Vec::new(),
                    };
                    new_ah.ah_key[..key_bytes.len()].copy_from_slice(key_bytes);
                    tp.entry(key_bytes.to_vec()).or_insert(new_ah)
                };
                cur_aff_key = Some((is_pref, key_bytes.to_vec()));

                if itemcnt > lasti && items[lasti].to_bytes() == b"S" {
                    lasti += 1;
                    cur_aff.ah_follows = 1;
                } else {
                    cur_aff.ah_follows = 0;
                }

                if itemcnt > lasti
                    && !aff.af_ignoreextra
                    && items[lasti].to_bytes().first() != Some(&b'#')
                {
                    smsg!(0, gettext(E_AFFTRAILING), fname, lnum, items[lasti]);
                }

                if item2_bytes != b"Y" && item2_bytes != b"N" {
                    smsg!(
                        0,
                        gettext(c"Expected Y or N in %s line %d: %s"),
                        fname,
                        lnum,
                        items[2],
                    );
                }

                if item0_bytes[0] == b'P' && aff.af_pfxpostpone != 0 {
                    if cur_aff.ah_new_id == 0 {
                        check_renumber(spin);
                        spin.si_newpref_id += 1;
                        cur_aff.ah_new_id = spin.si_newpref_id;
                        did_postpone_prefix = false;
                    } else {
                        did_postpone_prefix = true;
                    }
                }

                aff_todo = crate::charset::parse_i64_prefix(items[3].to_bytes()) as i32;
            } else if (item0_bytes == b"PFX" || item0_bytes == b"SFX")
                && aff_todo > 0
                && cur_aff_key
                    .as_ref()
                    .is_some_and(|(_, k)| k.as_slice() == items[1].to_bytes())
                && itemcnt >= 5
            {
                let lasti: usize = 5;
                if itemcnt > lasti
                    && items[lasti].to_bytes().first() != Some(&b'#')
                    && (items[lasti].to_bytes() != b"-" || itemcnt != lasti + 1)
                {
                    smsg!(0, gettext(E_AFFTRAILING), fname, lnum, items[lasti]);
                }

                aff_todo -= 1;
                let mut aff_entry_val = Affentry {
                    ae_chop: None,
                    ae_add: None,
                    ae_flags: None,
                    ae_cond: None,
                    ae_prog: None,
                    ae_compforbid: 0,
                    ae_comppermit: 0,
                };
                let aff_entry = &mut aff_entry_val;

                if items[2].to_bytes() != b"0" {
                    aff_entry.ae_chop = Some(items[2].to_owned());
                }
                if items[3].to_bytes() != b"0" {
                    let item3_bytes = items[3].to_bytes();
                    if let Some(slash_pos) = item3_bytes.iter().position(|&b| b == b'/') {
                        aff_entry.ae_add =
                            Some(std::ffi::CString::new(&item3_bytes[..slash_pos]).unwrap());
                        aff_entry.ae_flags =
                            Some(std::ffi::CString::new(&item3_bytes[slash_pos + 1..]).unwrap());
                        aff_process_flags(&aff, &mut *aff_entry);
                    } else {
                        aff_entry.ae_add = Some(std::ffi::CString::new(item3_bytes).unwrap());
                    }
                }

                if !spin.si_ascii
                    || ((*aff_entry)
                        .chop_cstr()
                        .is_none_or(|s| s.to_bytes().is_ascii())
                        && (*aff_entry)
                            .add_cstr()
                            .is_none_or(|s| s.to_bytes().is_ascii()))
                {
                    let (is_pref, ref aff_k) = *cur_aff_key.as_ref().unwrap();
                    let cur_aff_mut = if is_pref {
                        aff.af_pref.get_mut(aff_k.as_slice()).unwrap()
                    } else {
                        aff.af_suff.get_mut(aff_k.as_slice()).unwrap()
                    };
                    cur_aff_mut.ah_entries.insert(0, aff_entry_val);
                    let aff_entry = &mut cur_aff_mut.ah_entries[0];

                    if items[4].to_bytes() != b"." {
                        let mut buf = [0u8; MAXLINELEN];
                        aff_entry.ae_cond = Some(items[4].to_owned());
                        format_to_buf!(
                            &mut buf[..],
                            if item0_bytes.first() == Some(&b'P') {
                                c"^%s"
                            } else {
                                c"%s$"
                            },
                            items[4],
                        );
                        aff_entry.ae_prog = crate::regexp::vim_regcomp_box(
                            crate::memory::bytes_as_cstr(&buf),
                            RE_MAGIC + RE_STRING + RE_STRICT,
                        );
                        if aff_entry.ae_prog.is_none() {
                            smsg!(
                                0,
                                gettext(c"Broken condition in %s line %d: %s"),
                                fname,
                                lnum,
                                items[4],
                            );
                        }
                    }

                    if item0_bytes.first() == Some(&b'P')
                        && aff.af_pfxpostpone != 0
                        && aff_entry.ae_flags.is_none()
                    {
                        let mut upper = false;
                        if let (Some(chop), Some(add)) =
                            (aff_entry.chop_cstr(), aff_entry.add_cstr())
                            && chop.to_bytes().len() == utfc_ptr2len(chop) as usize
                        {
                            let c = utf_ptr2char(chop);
                            let c_up = spell_toupper(c);
                            if c_up != c
                                && aff_entry
                                    .cond_cstr()
                                    .is_none_or(|cond| utf_ptr2char(cond) == c)
                            {
                                let add_bytes = add.to_bytes();
                                let mut end_idx = add_bytes.len();
                                crate::mbyte::mb_idx_back(add_bytes, &mut end_idx);
                                if utf_slice2char(&add_bytes[end_idx..]) == c_up {
                                    upper = true;
                                    let new_add =
                                        std::ffi::CString::new(&add_bytes[..end_idx]).unwrap();
                                    aff_entry.ae_chop = None;
                                    aff_entry.ae_add = Some(new_add);

                                    if aff_entry.ae_cond.is_some() {
                                        let mut buf = [0u8; MAXLINELEN];
                                        onecap_copy(items[4].to_bytes(), &mut buf, true);
                                        let cond_cstr = crate::memory::cstring_until_nul(&buf);
                                        format_to_buf!(&mut buf[..], c"^%s", &cond_cstr,);
                                        aff_entry.ae_cond = Some(cond_cstr);
                                        aff_entry.ae_prog = crate::regexp::vim_regcomp_box(
                                            crate::memory::bytes_as_cstr(&buf),
                                            RE_MAGIC + RE_STRING,
                                        );
                                    }
                                }
                            }
                        }

                        if aff_entry.ae_chop.is_none() {
                            let prefcond = &mut spin.si_prefcond;
                            let mut idx = prefcond.len() as i32 - 1;
                            while idx >= 0 {
                                if prefcond[idx as usize].as_deref() == aff_entry.cond_cstr() {
                                    break;
                                }
                                idx -= 1;
                            }
                            if idx < 0 {
                                idx = prefcond.len() as i32;
                                prefcond.push(aff_entry.ae_cond.clone());
                            }

                            let p_add = aff_entry.ae_add.as_ref().map_or(&[][..], |s| s.to_bytes());

                            let mut n = PFX_FLAGS;
                            if cur_aff_mut.ah_combine == 0 {
                                n |= WFP_NC;
                            }
                            if upper {
                                n |= WFP_UP;
                            }
                            if aff_entry.ae_comppermit != 0 {
                                n |= WFP_COMPPERMIT;
                            }
                            if aff_entry.ae_compforbid != 0 {
                                n |= WFP_COMPFORBID;
                            }
                            tree_add_word(
                                spin,
                                p_add,
                                spin.si_prefroot,
                                n,
                                idx,
                                cur_aff_mut.ah_new_id,
                            );
                            did_postpone_prefix = true;
                        }

                        if aff_todo == 0 && !did_postpone_prefix {
                            spin.si_newpref_id -= 1;
                            cur_aff_mut.ah_new_id = 0;
                        }
                    }
                }
            } else if is_aff_rule(&items, itemcnt_i, c"FOL", 2) && !has_fol {
                has_fol = true;
            } else if is_aff_rule(&items, itemcnt_i, c"LOW", 2) && !has_low {
                has_low = true;
            } else if is_aff_rule(&items, itemcnt_i, c"UPP", 2) && !has_upp {
                has_upp = true;
            } else if is_aff_rule(&items, itemcnt_i, c"REP", 2)
                || is_aff_rule(&items, itemcnt_i, c"REPSAL", 2)
            {
                if !items[1].to_bytes().first().is_some_and(u8::is_ascii_digit) {
                    smsg!(
                        0,
                        gettext(c"Expected REP(SAL) count in %s line %d"),
                        fname,
                        lnum,
                    );
                }
            } else if (item0_bytes == b"REP" || item0_bytes == b"REPSAL") && itemcnt >= 3 {
                if itemcnt > 3 && items[3].to_bytes().first() != Some(&b'#') {
                    smsg!(0, gettext(E_AFFTRAILING), fname, lnum, items[3]);
                }
                let is_repsal = item0_bytes == b"REPSAL";
                if if is_repsal { do_repsal } else { do_rep } {
                    let p1: Vec<u8> = items[1]
                        .to_bytes()
                        .iter()
                        .map(|&b| if b == b'_' { b' ' } else { b })
                        .collect();
                    let p2: Vec<u8> = items[2]
                        .to_bytes()
                        .iter()
                        .map(|&b| if b == b'_' { b' ' } else { b })
                        .collect();
                    add_fromto(
                        if is_repsal {
                            &mut spin.si_repsal
                        } else {
                            &mut spin.si_rep
                        },
                        &p1,
                        &p2,
                    );
                }
            } else if is_aff_rule(&items, itemcnt_i, c"MAP", 2) {
                if !found_map {
                    found_map = true;
                    if !items[1].to_bytes().first().is_some_and(u8::is_ascii_digit) {
                        smsg!(0, gettext(c"Expected MAP count in %s line %d"), fname, lnum,);
                    }
                } else if do_mapline {
                    let si_map = &mut spin.si_map;
                    let item_cstr = items[1];
                    let item_bytes = item_cstr.to_bytes();
                    let mut rem = item_bytes;
                    while !rem.is_empty() {
                        let c = mb_ptr2char_adv(&mut rem);
                        let p_m = &item_cstr[item_bytes.len() - rem.len()..];
                        let in_map = if si_map.is_empty() {
                            false
                        } else {
                            si_map.push(0);
                            let found =
                                vim_strchr(crate::memory::bytes_as_cstr(si_map), c).is_some();
                            si_map.pop();
                            found
                        };
                        if in_map || vim_strchr(p_m, c).is_some() {
                            smsg!(
                                0,
                                gettext(c"Duplicate character in MAP in %s line %d"),
                                fname,
                                lnum,
                            );
                        }
                    }
                    si_map.extend_from_slice(items[1].to_bytes());
                    si_map.push(b'/');
                }
            } else if is_aff_rule(&items, itemcnt_i, c"SAL", 3) {
                if do_sal {
                    let item2_bytes = items[2].to_bytes();
                    if items[1].to_bytes() == b"followup" {
                        spin.si_followup = sal_to_bool(item2_bytes);
                    } else if items[1].to_bytes() == b"collapse_result" {
                        spin.si_collapse = sal_to_bool(item2_bytes);
                    } else if items[1].to_bytes() == b"remove_accents" {
                        spin.si_rem_accents = sal_to_bool(item2_bytes);
                    } else {
                        let to_s = if item2_bytes == b"_" {
                            &[][..]
                        } else {
                            item2_bytes
                        };
                        add_fromto(&mut spin.si_sal, items[1].to_bytes(), to_s);
                    }
                }
            } else if is_aff_rule(&items, itemcnt_i, c"SOFOFROM", 2) && sofofrom.is_none() {
                sofofrom = Some(items[1].to_owned());
            } else if is_aff_rule(&items, itemcnt_i, c"SOFOTO", 2) && sofoto.is_none() {
                sofoto = Some(items[1].to_owned());
            } else if item0_bytes == b"COMMON" {
                for &item in items.iter().take(itemcnt).skip(1) {
                    spin.si_commonwords.insert(item.to_bytes().to_vec());
                }
            } else {
                smsg!(
                    0,
                    gettext(c"Unrecognized or duplicate item in %s line %d: %s"),
                    fname,
                    lnum,
                    items[0],
                );
            }
        }
    }

    if (has_fol || has_low || has_upp) && spin.si_clear_chartab != 0 {
        init_spell_chartab();
        spin.si_clear_chartab = 0;
    }

    if compmax != 0 {
        aff_check_number(spin.si_compmax, compmax, c"COMPOUNDWORDMAX");
        spin.si_compmax = compmax;
    }

    if compminlen != 0 {
        aff_check_number(spin.si_compminlen, compminlen, c"COMPOUNDMIN");
        spin.si_compminlen = compminlen;
    }

    if compsylmax != 0 {
        if syllable.is_none() {
            smsg!(0, c"%s", gettext(c"COMPOUNDSYLMAX used without SYLLABLE"),);
        }
        aff_check_number(spin.si_compsylmax, compsylmax, c"COMPOUNDSYLMAX");
        spin.si_compsylmax = compsylmax;
    }

    if compoptions != 0 {
        aff_check_number(spin.si_compoptions, compoptions, c"COMPOUND options");
        spin.si_compoptions |= compoptions;
    }

    if let Some(ref cf) = compflags {
        process_compflags(spin, &mut aff, cf.to_bytes());
    }

    if spin.si_newcomp_id < spin.si_newpref_id {
        if spin.si_newcomp_id == 127 || spin.si_newcomp_id == 255 {
            msg(gettext(c"Too many postponed prefixes"), 0);
        } else if spin.si_newpref_id == 0 || spin.si_newpref_id == 127 {
            msg(gettext(c"Too many compound flags"), 0);
        } else {
            msg(
                gettext(c"Too many postponed prefixes and/or compound flags"),
                0,
            );
        }
    }

    if let Some(syl) = syllable {
        aff_check_string(spin.si_syllable.as_deref(), syl.as_c_str(), c"SYLLABLE");
        spin.si_syllable = Some(syl);
    }

    match (sofofrom, sofoto) {
        (None, None) => {}
        (Some(_), None) => {
            smsg!(0, gettext(c"Missing SOFOTO line in %s"), fname,);
        }
        (None, Some(_)) => {
            smsg!(0, gettext(c"Missing SOFOFROM line in %s"), fname,);
        }
        (Some(sf_from), Some(sf_to)) => {
            if !spin.si_sal.is_empty() {
                smsg!(0, gettext(c"Both SAL and SOFO lines in %s"), fname);
            } else {
                aff_check_string(spin.si_sofofr.as_deref(), sf_from.as_c_str(), c"SOFOFROM");
                aff_check_string(spin.si_sofoto.as_deref(), sf_to.as_c_str(), c"SOFOTO");
                spin.si_sofofr = Some(sf_from);
                spin.si_sofoto = Some(sf_to);
            }
        }
    }

    if let Some(mw) = midword {
        aff_check_string(spin.si_midword.as_deref(), mw.as_c_str(), c"MIDWORD");
        spin.si_midword = Some(mw);
    }

    Some(aff)
}

fn is_aff_rule(
    items: &[&std::ffi::CStr],
    itemcnt: i32,
    rulename: &std::ffi::CStr,
    mincount: i32,
) -> bool {
    items[0] == rulename
        && (itemcnt == mincount
            || (itemcnt > mincount && items[mincount as usize].to_bytes().first() == Some(&b'#')))
}

fn aff_process_flags(affile: &Afffile, entry: &mut Affentry) {
    if let Some(ref flags_cstr) = entry.ae_flags
        && (affile.af_compforbid != 0 || affile.af_comppermit != 0)
    {
        let flags_bytes = flags_cstr.to_bytes();
        let mut out_bytes = Vec::with_capacity(flags_bytes.len());
        let mut rem = flags_bytes;
        while !rem.is_empty() {
            let prev = rem;
            let flag = get_affitem(affile.af_flagtype, &mut rem);
            let keep = if flag == affile.af_comppermit {
                entry.ae_comppermit = 1;
                false
            } else if flag == affile.af_compforbid {
                entry.ae_compforbid = 1;
                false
            } else {
                true
            };
            if affile.af_flagtype == AFT_NUM && rem.first() == Some(&b',') {
                rem = &rem[1..];
            }
            if keep {
                out_bytes.extend_from_slice(&prev[..prev.len() - rem.len()]);
            }
        }
        if out_bytes.is_empty() {
            entry.ae_flags = None;
        } else {
            entry.ae_flags = std::ffi::CString::new(out_bytes).ok();
        }
    }
}

fn spell_info_item(s: &[u8]) -> bool {
    matches!(
        s,
        b"NAME" | b"HOME" | b"VERSION" | b"AUTHOR" | b"EMAIL" | b"COPYRIGHT"
    )
}

fn affitem2flag(
    flagtype: i32,
    item_cstr: &std::ffi::CStr,
    fname_cstr: &std::ffi::CStr,
    lnum: i32,
) -> u32 {
    let mut rem = item_cstr.to_bytes();
    let res = get_affitem(flagtype, &mut rem);
    if res == 0 {
        if flagtype == AFT_NUM {
            smsg!(
                0,
                gettext(c"Flag is not a number in %s line %d: %s"),
                fname_cstr,
                lnum,
                item_cstr,
            );
        } else {
            smsg!(
                0,
                gettext(c"Illegal flag in %s line %d: %s"),
                fname_cstr,
                lnum,
                item_cstr,
            );
        }
    }
    if !rem.is_empty() {
        smsg!(0, gettext(E_AFFNAME), fname_cstr, lnum, item_cstr);
        return 0;
    }
    res
}

fn get_affitem(flagtype: i32, pp: &mut &[u8]) -> u32 {
    let mut res: i32;
    if flagtype == AFT_NUM {
        if !pp.first().is_some_and(u8::is_ascii_digit) {
            if !pp.is_empty() {
                *pp = &pp[1..];
            }
            return 0;
        }
        res = getdigits_int(pp, true, 0);
        if res == 0 {
            res = ZERO_FLAG as i32;
        }
    } else {
        res = mb_ptr2char_adv(pp);
        if flagtype == AFT_LONG
            || (flagtype == AFT_CAPLONG && res >= b'A' as i32 && res <= b'Z' as i32)
        {
            if pp.is_empty() {
                return 0;
            }
            res = mb_ptr2char_adv(pp) + (res << 16);
        }
    }
    res as u32
}

fn process_compflags(spin: &mut Spellinfo, aff: &mut Afffile, compflags: &[u8]) {
    let mut bytes = spin
        .si_compflags
        .as_ref()
        .map_or_else(Vec::new, |s| s.to_bytes().to_vec());
    if !bytes.is_empty() {
        bytes.push(b'/');
    }

    let mut cur = crate::memory::bytes_until_nul(compflags);
    while !cur.is_empty() {
        if matches!(cur[0], b'/' | b'?' | b'*' | b'+' | b'[' | b']') {
            bytes.push(cur[0]);
            cur = &cur[1..];
        } else {
            let prev = cur;
            let flag = get_affitem(aff.af_flagtype, &mut cur);
            if flag != 0 {
                let key_len = (prev.len() - cur.len()).min(AH_KEY_LEN - 1);
                let key_bytes = &prev[..key_len];
                let id = if let Some(ci) = aff.af_comp.get(key_bytes) {
                    ci.ci_new_id
                } else {
                    let mut ci = Compitem {
                        ci_key: [0; AH_KEY_LEN],
                        ci_flag: flag,
                        ci_new_id: 0,
                    };
                    ci.ci_key[..key_len].copy_from_slice(key_bytes);
                    let mut new_id: i32;
                    loop {
                        check_renumber(spin);
                        new_id = spin.si_newcomp_id;
                        spin.si_newcomp_id -= 1;
                        if vim_strchr(c"/?*+[]\\-^", new_id)
                            .map_or(std::ptr::null_mut(), |s| s.as_ptr().cast_mut())
                            .is_null()
                        {
                            break;
                        }
                    }
                    ci.ci_new_id = new_id;
                    aff.af_comp.insert(key_bytes.to_vec(), ci);
                    new_id
                };
                bytes.push(id as u8);
            }
            if aff.af_flagtype == AFT_NUM && cur.first() == Some(&b',') {
                cur = &cur[1..];
            }
        }
    }

    spin.si_compflags = Some(std::ffi::CString::new(bytes).unwrap());
}

fn check_renumber(spin: &mut Spellinfo) {
    if spin.si_newpref_id == spin.si_newcomp_id && spin.si_newcomp_id < 128 {
        spin.si_newpref_id = 127;
        spin.si_newcomp_id = 255;
    }
}

fn store_afflist_add(store_afflist: &mut [u8], cntp: &mut i32, id: i32) -> bool {
    if *cntp >= (MAXWLEN - 1) as i32 {
        emsg(gettext(E_TOO_MANY_POSTPONED_PREFIXES_SPELL));
        return false;
    }
    store_afflist[*cntp as usize] = id as u8;
    *cntp += 1;
    store_afflist[*cntp as usize] = 0;
    true
}

fn flag_in_afflist(flagtype: i32, afflist: &[u8], flag: u32) -> bool {
    let aff_bytes = crate::memory::bytes_until_nul(afflist);
    match flagtype {
        AFT_CHAR => aff_bytes.contains(&(flag as u8)),
        AFT_CAPLONG | AFT_LONG => {
            let mut rem = aff_bytes;
            while !rem.is_empty() {
                let mut n = mb_ptr2char_adv(&mut rem) as u32;
                if (flagtype == AFT_LONG || (n >= b'A' as u32 && n <= b'Z' as u32))
                    && !rem.is_empty()
                {
                    n = (mb_ptr2char_adv(&mut rem) as u32) + (n << 16);
                }
                if n == flag {
                    return true;
                }
            }
            false
        }
        AFT_NUM => {
            let mut rem = aff_bytes;
            while !rem.is_empty() {
                let digits = getdigits_int(&mut rem, true, 0);
                debug_assert!(digits >= 0);
                let mut n = digits as u32;
                if n == 0 {
                    n = ZERO_FLAG;
                }
                if n == flag {
                    return true;
                }
                if !rem.is_empty() {
                    rem = &rem[1..];
                }
            }
            false
        }
        _ => false,
    }
}

fn aff_check_number(spinval: i32, affval: i32, name: &std::ffi::CStr) {
    if spinval != 0 && spinval != affval {
        smsg!(
            0,
            gettext(c"%s value differs from what is used in another .aff file"),
            name,
        );
    }
}

fn aff_check_string(
    spinval: Option<&std::ffi::CStr>,
    affval: &std::ffi::CStr,
    name: &std::ffi::CStr,
) {
    if let Some(s) = spinval
        && s != affval
    {
        smsg!(
            0,
            gettext(c"%s value differs from what is used in another .aff file"),
            name,
        );
    }
}

fn add_fromto(gap: &mut Vec<Fromto>, from: &[u8], to: &[u8]) {
    let mut word = [0u8; MAXWLEN];
    spell_casefold(
        &curwin_mut(),
        crate::memory::bytes_until_nul(from),
        &mut word,
    );
    let ft_from = crate::memory::bytes_as_cstr(&word).to_owned();
    spell_casefold(&curwin_mut(), crate::memory::bytes_until_nul(to), &mut word);
    let ft_to = crate::memory::bytes_as_cstr(&word).to_owned();
    gap.push(Fromto { ft_from, ft_to });
}

fn sal_to_bool(s: &[u8]) -> bool {
    s == b"1" || s == b"true"
}

fn spell_read_dic(spin: &mut Spellinfo, fname_cstr: &std::ffi::CStr, affile: &Afffile) -> bool {
    let fname = fname_cstr;
    let mut ht: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
    let mut line = [0u8; MAXLINELEN];
    let mut store_afflist = [0u8; MAXWLEN];
    let mut lnum: i32 = 1;
    let mut non_ascii: i32 = 0;
    let mut retval: bool = true;
    let mut message = [0u8; MAXLINELEN + MAXWLEN];
    let mut duplicate: i32 = 0;
    let mut last_msg_time: Timestamp = 0;

    let Some(fd) = CFile::open(fname, c"r") else {
        semsg!(gettext(E_NOTOPEN), fname);
        return false;
    };

    format_to_buf!(
        &mut *g_st().iobuff_bytes_mut(),
        gettext(c"Reading dictionary file %s..."),
        fname,
    );
    spell_message(&*spin, &g_st().iobuff_cstr());

    spin.si_msg_count = 999999;

    if vim_fgets(&mut line, &fd)
        || !ascii_isdigit(
            crate::charset::skipwhite_slice(crate::memory::bytes_until_nul(&line))
                .first()
                .copied()
                .unwrap_or(0),
        )
    {
        semsg!(gettext(c"E760: No word count in %s"), fname);
    }

    while !vim_fgets(&mut line, &fd) && !g_st().got_int.get() {
        line_breakcheck();
        lnum += 1;
        if line[0] == b'#' || line[0] == b'/' {
            continue;
        }
        let mut l = crate::memory::bytes_until_nul(&line).len();
        while l > 0 && line[l - 1] <= b' ' {
            l -= 1;
        }
        if l == 0 {
            continue;
        }
        line[l] = 0;

        let mut pc_vec: Vec<u8>;
        let w_slice: &mut [u8] = if spin.si_conv.vc_type != CONV_NONE {
            let Some(pc_bytes) =
                string_convert(&spin.si_conv, crate::memory::bytes_until_nul(&line), None)
            else {
                smsg!(
                    0,
                    gettext(c"Conversion failure for word in %s line %d: %s"),
                    fname,
                    lnum,
                    crate::memory::bytes_as_cstr(&line),
                );
                continue;
            };
            pc_vec = pc_bytes;
            &mut pc_vec
        } else {
            &mut line[..=l]
        };

        let mut w_len = crate::memory::bytes_until_nul(w_slice).len();
        let mut afflist_off: Option<usize> = None;
        let mut idx = 0;
        while idx < w_len {
            if w_slice[idx] == b'\\'
                && idx + 1 < w_len
                && (w_slice[idx + 1] == b'\\' || w_slice[idx + 1] == b'/')
            {
                w_slice.copy_within(idx + 1..=w_len, idx);
                w_len -= 1;
            } else if w_slice[idx] == b'/' {
                w_slice[idx] = 0;
                afflist_off = Some(idx + 1);
                w_len = idx;
                break;
            }
            idx += crate::mbyte::utfc_slice2len(&w_slice[idx..w_len]);
        }
        let w_cstr = crate::memory::bytes_as_cstr(w_slice);
        let w_bytes = w_cstr.to_bytes();

        if spin.si_ascii && !w_bytes.is_ascii() {
            non_ascii += 1;
            continue;
        }

        if spin.si_verbose && spin.si_msg_count > 10000 {
            spin.si_msg_count = 0;
            if os_time() > last_msg_time {
                last_msg_time = os_time();
                format_to_buf!(
                    &mut message[..],
                    gettext(c"line %6d, word %6d - %s"),
                    lnum,
                    spin.si_foldwcount + spin.si_keepwcount,
                    w_cstr,
                );
                msg_start();
                msg_outtrans_long(crate::memory::bytes_as_cstr(&message), 0);
                msg_clr_eos();
                g_st().msg_didout.set(false);
                g_st().msg_col.set(0);
                ui_flush();
            }
        }

        let dw_bytes = w_bytes.to_vec();
        if !ht.insert(dw_bytes) {
            if opt_vars().p_verbose.get() > 0 {
                smsg!(
                    0,
                    gettext(c"Duplicate word in %s line %d: %s"),
                    fname,
                    lnum,
                    w_cstr,
                );
            } else if duplicate == 0 {
                smsg!(
                    0,
                    gettext(c"First duplicate word in %s line %d: %s"),
                    fname,
                    lnum,
                    w_cstr,
                );
            }
            duplicate += 1;
        }

        let mut flags: i32 = 0;
        store_afflist[0] = 0;
        let mut pfxlen: i32 = 0;
        let mut totlen: i32 = 0;
        let mut need_affix = false;
        let aff_bytes: &[u8] = if let Some(off) = afflist_off {
            crate::memory::bytes_until_nul(&w_slice[off..])
        } else {
            &[]
        };
        if afflist_off.is_some() {
            flags |= get_affix_flags(affile, aff_bytes);

            if affile.af_needaffix != 0
                && flag_in_afflist(affile.af_flagtype, aff_bytes, affile.af_needaffix)
            {
                need_affix = true;
            }

            if affile.af_pfxpostpone != 0 {
                if !get_pfxlist(affile, aff_bytes, &mut store_afflist, &mut totlen) {
                    retval = false;
                    break;
                }
                pfxlen = totlen;
            }

            if spin.si_compflags.is_some()
                && !get_compflags(affile, aff_bytes, &mut store_afflist, &mut totlen)
            {
                retval = false;
                break;
            }
        }

        if !store_word(
            spin,
            w_bytes,
            flags,
            spin.si_region,
            &store_afflist,
            need_affix,
        ) {
            retval = false;
        }

        if afflist_off.is_some() {
            if !store_aff_word(
                spin,
                w_bytes,
                aff_bytes,
                affile,
                &affile.af_suff,
                Some(&affile.af_pref),
                CONDIT_SUF,
                flags,
                &store_afflist,
                pfxlen,
            ) {
                retval = false;
            }

            if !store_aff_word(
                spin,
                w_bytes,
                aff_bytes,
                affile,
                &affile.af_pref,
                None,
                CONDIT_SUF,
                flags,
                &store_afflist,
                pfxlen,
            ) {
                retval = false;
            }
        }
    }

    if duplicate > 0 {
        smsg!(0, gettext(c"%d duplicate word(s) in %s"), duplicate, fname,);
    }
    if spin.si_ascii && non_ascii > 0 {
        smsg!(
            0,
            gettext(c"Ignored %d word(s) with non-ASCII characters in %s"),
            non_ascii,
            fname,
        );
    }

    spell_close(fd);
    retval
}

fn get_affix_flags(affile: &Afffile, afflist: &[u8]) -> i32 {
    let mut flags: i32 = 0;

    if affile.af_keepcase != 0 && flag_in_afflist(affile.af_flagtype, afflist, affile.af_keepcase) {
        flags |= WF_KEEPCAP | WF_FIXCAP;
    }
    if affile.af_rare != 0 && flag_in_afflist(affile.af_flagtype, afflist, affile.af_rare) {
        flags |= WF_RARE;
    }
    if affile.af_bad != 0 && flag_in_afflist(affile.af_flagtype, afflist, affile.af_bad) {
        flags |= WF_BANNED;
    }
    if affile.af_needcomp != 0 && flag_in_afflist(affile.af_flagtype, afflist, affile.af_needcomp) {
        flags |= WF_NEEDCOMP;
    }
    if affile.af_comproot != 0 && flag_in_afflist(affile.af_flagtype, afflist, affile.af_comproot) {
        flags |= WF_COMPROOT;
    }
    if affile.af_nosuggest != 0 && flag_in_afflist(affile.af_flagtype, afflist, affile.af_nosuggest)
    {
        flags |= WF_NOSUGGEST;
    }
    flags
}

fn get_pfxlist(affile: &Afffile, afflist: &[u8], store_afflist: &mut [u8], cntp: &mut i32) -> bool {
    let mut rem = crate::memory::bytes_until_nul(afflist);
    while !rem.is_empty() {
        let prev = rem;
        if get_affitem(affile.af_flagtype, &mut rem) != 0 {
            let key_len = (prev.len() - rem.len()).min(AH_KEY_LEN - 1);
            let key_bytes = &prev[..key_len];
            if let Some(ah) = affile.af_pref.get(key_bytes) {
                let id = ah.ah_new_id;
                if id != 0 && !store_afflist_add(store_afflist, cntp, id) {
                    return false;
                }
            }
        }
        if affile.af_flagtype == AFT_NUM && rem.first() == Some(&b',') {
            rem = &rem[1..];
        }
    }
    true
}

fn get_compflags(
    affile: &Afffile,
    afflist: &[u8],
    store_afflist: &mut [u8],
    cntp: &mut i32,
) -> bool {
    let mut rem = crate::memory::bytes_until_nul(afflist);
    while !rem.is_empty() {
        let prev = rem;
        if get_affitem(affile.af_flagtype, &mut rem) != 0 {
            let key_len = (prev.len() - rem.len()).min(AH_KEY_LEN - 1);
            let key_bytes = &prev[..key_len];
            if let Some(ci) = affile.af_comp.get(key_bytes)
                && !store_afflist_add(store_afflist, &mut *cntp, ci.ci_new_id)
            {
                return false;
            }
        }
        if affile.af_flagtype == AFT_NUM && rem.first() == Some(&b',') {
            rem = &rem[1..];
        }
    }
    true
}

fn store_aff_word(
    spin: &mut Spellinfo,
    word_in: &[u8],
    afflist_in: &[u8],
    affile: &Afffile,
    ht: &std::collections::BTreeMap<Vec<u8>, Affheader>,
    xht: Option<&std::collections::BTreeMap<Vec<u8>, Affheader>>,
    condit: i32,
    flags: i32,
    pfxlist_in: &[u8],
    pfxlen: i32,
) -> bool {
    let word_cstr = crate::memory::bytes_to_cstring(crate::memory::bytes_until_nul(word_in));
    let afflist = crate::memory::bytes_until_nul(afflist_in);
    let pfxlist_bytes = crate::memory::bytes_until_nul(pfxlist_in);
    let mut newword = [0u8; MAXWLEN];
    let mut retval = true;
    let mut store_afflist = [0u8; MAXWLEN];
    let mut pfx_pfxlist = [0u8; MAXWLEN];
    let wordlen = word_cstr.to_bytes().len();

    for ah in ht.values() {
        if !retval {
            break;
        }
        if ((condit & CONDIT_COMB) == 0 || ah.ah_combine != 0)
            && flag_in_afflist(affile.af_flagtype, afflist, ah.ah_flag)
        {
            for ae in &ah.ah_entries {
                let ae_chop = ae.chop_cstr();
                let ae_add = ae.add_cstr();
                let ae_flags = ae.flags_cstr();
                if (xht.is_some()
                    || affile.af_pfxpostpone == 0
                    || ae_chop.is_some()
                    || ae_flags.is_some())
                    && ae_chop.is_none_or(|c| c.to_bytes().len() < wordlen)
                    && (ae.ae_prog.is_none()
                        || vim_regexec_prog(ae.ae_prog.as_deref(), false, &word_cstr, 0))
                    && (((condit & CONDIT_CFIX) == 0)
                        == ((condit & CONDIT_AFF) == 0
                            || ae_flags.is_none_or(|fl| {
                                !flag_in_afflist(
                                    affile.af_flagtype,
                                    fl.to_bytes(),
                                    affile.af_circumfix,
                                )
                            })))
                {
                    if xht.is_none() {
                        if let Some(ae_add) = ae_add {
                            crate::memory::copy_bytes_to_c_buf(&mut newword, ae_add.to_bytes());
                        } else {
                            newword[0] = 0;
                        }
                        let word_bytes = word_cstr.to_bytes();
                        let mut p_off = 0usize;
                        if let Some(ae_chop) = ae_chop {
                            let mut i = mb_charlen(ae_chop.to_bytes());
                            while i > 0 {
                                p_off += crate::mbyte::utfc_ptr2len(
                                    word_bytes.get(p_off..).unwrap_or(&[]),
                                ) as usize;
                                i -= 1;
                            }
                        }
                        crate::memory::append_bytes_to_c_buf(
                            &mut newword,
                            word_bytes.get(p_off..).unwrap_or(&[]),
                        );
                    } else {
                        crate::memory::copy_bytes_to_c_buf(&mut newword, word_cstr.to_bytes());
                        if let Some(ae_chop) = ae_chop {
                            let nw_bytes = crate::memory::bytes_until_nul(&newword);
                            let mut end_idx = nw_bytes.len();
                            let mut i = mb_charlen(ae_chop.to_bytes());
                            while i > 0 {
                                crate::mbyte::mb_idx_back(nw_bytes, &mut end_idx);
                                i -= 1;
                            }
                            newword[end_idx] = 0;
                        }
                        if let Some(ae_add) = ae_add {
                            crate::memory::append_bytes_to_c_buf(&mut newword, ae_add.to_bytes());
                        }
                    }

                    let mut use_flags = flags;
                    let mut use_pfx_slice = pfxlist_bytes;
                    let mut use_pfxlen = pfxlen;
                    let mut need_affix = false;
                    let mut use_condit = condit | CONDIT_COMB | CONDIT_AFF;
                    if let Some(ae_flags) = ae_flags {
                        let ae_flags_bytes = ae_flags.to_bytes();
                        use_flags |= get_affix_flags(affile, ae_flags_bytes);

                        if affile.af_needaffix != 0
                            && flag_in_afflist(
                                affile.af_flagtype,
                                ae_flags_bytes,
                                affile.af_needaffix,
                            )
                        {
                            need_affix = true;
                        }

                        if affile.af_circumfix != 0
                            && flag_in_afflist(
                                affile.af_flagtype,
                                ae_flags_bytes,
                                affile.af_circumfix,
                            )
                        {
                            use_condit |= CONDIT_CFIX;
                            if (condit & CONDIT_CFIX) == 0 {
                                need_affix = true;
                            }
                        }

                        if affile.af_pfxpostpone != 0 || spin.si_compflags.is_some() {
                            let mut listlen: i32 = 0;

                            if affile.af_pfxpostpone != 0 {
                                if !get_pfxlist(
                                    affile,
                                    ae_flags_bytes,
                                    &mut store_afflist,
                                    &mut listlen,
                                ) {
                                    retval = false;
                                    break;
                                }
                                use_pfxlen = listlen;
                            } else {
                                use_pfxlen = 0;
                            }

                            for &pi in &pfxlist_bytes[..(pfxlen as usize).min(pfxlist_bytes.len())]
                            {
                                if !store_afflist[..use_pfxlen as usize].contains(&pi)
                                    && !store_afflist_add(
                                        &mut store_afflist,
                                        &mut listlen,
                                        i32::from(pi),
                                    )
                                {
                                    retval = false;
                                    break;
                                }
                                use_pfxlen = listlen;
                            }
                            if !retval {
                                break;
                            }

                            if spin.si_compflags.is_some() {
                                if !get_compflags(
                                    affile,
                                    ae_flags_bytes,
                                    &mut store_afflist,
                                    &mut listlen,
                                ) {
                                    retval = false;
                                    break;
                                }
                            } else {
                                store_afflist[use_pfxlen as usize] = 0;
                            }
                            if !retval {
                                break;
                            }

                            for &pi in &pfxlist_bytes[(pfxlen as usize).min(pfxlist_bytes.len())..]
                            {
                                if !store_afflist[use_pfxlen as usize..listlen as usize]
                                    .contains(&pi)
                                    && !store_afflist_add(
                                        &mut store_afflist,
                                        &mut listlen,
                                        i32::from(pi),
                                    )
                                {
                                    retval = false;
                                    break;
                                }
                            }
                            if !retval {
                                break;
                            }
                            use_pfx_slice = crate::memory::bytes_until_nul(&store_afflist);
                        }
                    }

                    if !use_pfx_slice.is_empty() && ae.ae_compforbid != 0 {
                        let plen = (use_pfxlen as usize).min(use_pfx_slice.len());
                        pfx_pfxlist[..plen].copy_from_slice(&use_pfx_slice[..plen]);
                        pfx_pfxlist[plen] = 0;
                        use_pfx_slice = &pfx_pfxlist[..plen];
                    }

                    if spin
                        .si_prefroot
                        .is_some_and(|r| (&spin.si_nodes)[r].wn_sibling.is_some())
                    {
                        use_flags |= WF_HAS_AFF;
                        if ah.ah_combine == 0 {
                            use_pfx_slice =
                                &use_pfx_slice[(use_pfxlen as usize).min(use_pfx_slice.len())..];
                        }
                    }

                    if spin.si_compflags.is_some() && ae.ae_comppermit == 0 {
                        if xht.is_some() {
                            use_flags |= WF_NOCOMPAFT;
                        } else {
                            use_flags |= WF_NOCOMPBEF;
                        }
                    }

                    if !store_word(
                        spin,
                        &newword,
                        use_flags,
                        spin.si_region,
                        use_pfx_slice,
                        need_affix,
                    ) {
                        retval = false;
                    }

                    if (condit & CONDIT_SUF) != 0
                        && let Some(ae_flags) = ae_flags
                    {
                        let next_condit = use_condit & if xht.is_none() { !0 } else { !CONDIT_SUF };
                        if !store_aff_word(
                            spin,
                            &newword,
                            ae_flags.to_bytes(),
                            affile,
                            &affile.af_suff,
                            xht,
                            next_condit,
                            use_flags,
                            use_pfx_slice,
                            pfxlen,
                        ) {
                            retval = false;
                        }
                    }

                    if let Some(xht_ref) = xht
                        && ah.ah_combine != 0
                        && (!store_aff_word(
                            spin,
                            &newword,
                            afflist,
                            affile,
                            xht_ref,
                            None,
                            use_condit,
                            use_flags,
                            use_pfx_slice,
                            pfxlen,
                        ) || ae_flags.is_some_and(|fl| {
                            !store_aff_word(
                                spin,
                                &newword,
                                fl.to_bytes(),
                                affile,
                                xht_ref,
                                None,
                                use_condit,
                                use_flags,
                                use_pfx_slice,
                                pfxlen,
                            )
                        }))
                    {
                        retval = false;
                    }
                }
            }
        }
    }

    retval
}

fn spell_read_wordfile(spin: &mut Spellinfo, fname_cstr: &std::ffi::CStr) -> bool {
    let fname = fname_cstr;
    {
        let mut lnum: Linenr = 0;
        let mut rline = [0u8; MAXLINELEN];
        let mut pc_vec: Vec<u8>;
        let mut retval = true;
        let mut did_word = false;
        let mut non_ascii: i32 = 0;

        let Some(fd) = CFile::open(fname, c"r") else {
            semsg!(gettext(E_NOTOPEN), fname);
            return false;
        };

        format_to_buf!(
            &mut *g_st().iobuff_bytes_mut(),
            gettext(c"Reading word file %s..."),
            fname,
        );
        spell_message(&*spin, &g_st().iobuff_cstr());

        while !vim_fgets(&mut rline, &fd) && !g_st().got_int.get() {
            line_breakcheck();
            lnum += 1;

            if rline[0] == b'#' {
                continue;
            }

            let mut l = crate::memory::bytes_until_nul(&rline).len();
            while l > 0 && rline[l - 1] <= b' ' {
                l -= 1;
            }
            if l == 0 {
                continue;
            }
            rline[l] = 0;

            let line_buf: &[u8] = if spin.si_conv.vc_type != CONV_NONE {
                let Some(pc_bytes) =
                    string_convert(&spin.si_conv, crate::memory::bytes_until_nul(&rline), None)
                else {
                    smsg!(
                        0,
                        gettext(c"Conversion failure for word in %s line %d: %s"),
                        fname,
                        lnum,
                        crate::memory::bytes_as_cstr(&rline),
                    );
                    continue;
                };
                pc_vec = pc_bytes;
                &pc_vec
            } else {
                &rline
            };
            let full_line_cstr = crate::memory::bytes_as_cstr(line_buf);

            if full_line_cstr.to_bytes().first() == Some(&b'/') {
                let after_slash_cstr = &full_line_cstr[1..];
                let line_bytes = after_slash_cstr.to_bytes();
                if line_bytes.starts_with(b"encoding=") {
                    if spin.si_conv.vc_type != CONV_NONE {
                        smsg!(
                            0,
                            gettext(c"Duplicate /encoding= line ignored in %s line %d: %s",),
                            fname,
                            lnum,
                            full_line_cstr,
                        );
                    } else if did_word {
                        smsg!(
                            0,
                            gettext(c"/encoding= line after word ignored in %s line %d: %s",),
                            fname,
                            lnum,
                            full_line_cstr,
                        );
                    } else {
                        let line_cstr = &after_slash_cstr[9..];
                        let enc = enc_canonize(line_cstr);
                        if !spin.si_ascii
                            && !convert_setup(
                                &mut spin.si_conv,
                                Some(enc.as_c_str()),
                                Some(opt_vars().p_enc.as_cstr()),
                            )
                        {
                            smsg!(
                                0,
                                gettext(c"Conversion in %s not supported: from %s to %s"),
                                fname,
                                line_cstr,
                                &opt_vars().p_enc,
                            );
                        }
                        spin.si_conv.vc_fail = true;
                    }
                    continue;
                }

                if line_bytes.starts_with(b"regions=") {
                    if spin.si_region_count > 1 {
                        smsg!(
                            0,
                            gettext(c"Duplicate /regions= line ignored in %s line %d: %s"),
                            fname,
                            lnum,
                            after_slash_cstr,
                        );
                    } else {
                        let regions_cstr = &after_slash_cstr[8..];
                        if line_bytes.len() - 8 > MAXREGIONS * 2 {
                            smsg!(
                                0,
                                gettext(c"Too many regions in %s line %d: %s"),
                                fname,
                                lnum,
                                regions_cstr,
                            );
                        } else {
                            let reg_bytes = regions_cstr.to_bytes_with_nul();
                            spin.si_region_count = ((reg_bytes.len() - 1) / 2) as i32;
                            spin.si_region_name[..reg_bytes.len()].copy_from_slice(reg_bytes);
                            spin.si_region = (1 << spin.si_region_count) - 1;
                        }
                    }
                    continue;
                }

                smsg!(
                    0,
                    gettext(c"/ line ignored in %s line %d: %s"),
                    fname,
                    lnum,
                    full_line_cstr,
                );
                continue;
            }

            let mut flags: i32 = 0;
            let mut regionmask = spin.si_region;

            let line_bytes = full_line_cstr.to_bytes();
            let word_bytes = if let Some(slash_pos) = line_bytes.iter().position(|&b| b == b'/') {
                let flag_cstr = &full_line_cstr[slash_pos + 1..];
                let flag_bytes = flag_cstr.to_bytes();
                for (fi, &fb) in flag_bytes.iter().enumerate() {
                    if fb == b'=' {
                        flags |= WF_KEEPCAP | WF_FIXCAP;
                    } else if fb == b'!' {
                        flags |= WF_BANNED;
                    } else if fb == b'?' {
                        flags |= WF_RARE;
                    } else if fb.is_ascii_digit() {
                        if (flags & WF_REGION) == 0 {
                            regionmask = 0;
                        }
                        flags |= WF_REGION;

                        let r = (fb - b'0') as i32;
                        if r == 0 || r > spin.si_region_count {
                            smsg!(
                                0,
                                gettext(c"Invalid region nr in %s line %d: %s"),
                                fname,
                                lnum,
                                &flag_cstr[fi..],
                            );
                            break;
                        }
                        regionmask |= 1 << (r - 1);
                    } else {
                        smsg!(
                            0,
                            gettext(c"Unrecognized flags in %s line %d: %s"),
                            fname,
                            lnum,
                            &flag_cstr[fi..],
                        );
                        break;
                    }
                }
                &line_bytes[..slash_pos]
            } else {
                line_bytes
            };

            if spin.si_ascii && !word_bytes.is_ascii() {
                non_ascii += 1;
                continue;
            }

            if !store_word(spin, word_bytes, flags, regionmask, &[], false) {
                retval = false;
                break;
            }
            did_word = true;
        }

        spell_close(fd);

        if spin.si_ascii && non_ascii > 0 {
            format_to_buf!(
                &mut *g_st().iobuff_bytes_mut(),
                gettext(c"Ignored %d words with non-ASCII characters"),
                non_ascii,
            );
            spell_message(&*spin, &g_st().iobuff_cstr());
        }

        retval
    }
}

fn wordtree_alloc(spin: &mut Spellinfo) -> Option<usize> {
    Some(get_wordnode(spin))
}

fn valid_spell_word(bytes: &[u8]) -> bool {
    if !utf_valid_string(bytes) {
        return false;
    }
    let mut i = 0usize;
    while i < bytes.len() && bytes[i] != 0 {
        if bytes[i] < b' ' || (bytes[i] == b'/' && bytes.get(i + 1).is_none_or(|&b| b == 0)) {
            return false;
        }
        i += crate::mbyte::utfc_slice2len(&bytes[i..]);
    }
    true
}

fn store_word(
    spin: &mut Spellinfo,
    word: &[u8],
    flags: i32,
    region: i32,
    pfxlist: &[u8],
    need_affix: bool,
) -> bool {
    let word_bytes = crate::memory::bytes_until_nul(word);
    let pfx_bytes = crate::memory::bytes_until_nul(pfxlist);
    let ct = captype(word_bytes);
    let mut foldword = [0u8; MAXWLEN];
    let mut res = true;

    if !valid_spell_word(word_bytes) {
        return false;
    }

    spell_casefold(&curwin_mut(), word_bytes, &mut foldword);
    let mut p = pfx_bytes;
    while res {
        if !need_affix || !p.is_empty() {
            res = tree_add_word(
                spin,
                &foldword,
                spin.si_foldroot,
                ct | flags,
                region,
                p.first().copied().unwrap_or(0) as i32,
            );
        }
        if p.is_empty() {
            break;
        }
        p = &p[1..];
    }
    spin.si_foldwcount += 1;

    if res && (ct == WF_KEEPCAP || (flags & WF_KEEPCAP) != 0) {
        let mut p2 = pfx_bytes;
        while res {
            if !need_affix || !p2.is_empty() {
                res = tree_add_word(
                    spin,
                    word_bytes,
                    spin.si_keeproot,
                    flags,
                    region,
                    p2.first().copied().unwrap_or(0) as i32,
                );
            }
            if p2.is_empty() {
                break;
            }
            p2 = &p2[1..];
        }
        spin.si_keepwcount += 1;
    }
    res
}

fn tree_add_word(
    spin: &mut Spellinfo,
    word: &[u8],
    root: Option<usize>,
    flags: i32,
    region: i32,
    affix_id: i32,
) -> bool {
    let word_bytes = crate::memory::bytes_until_nul(word);
    let mut node = root;
    let mut prev: Option<NodeSlot> = None;

    let mut i: usize = 0;
    loop {
        if let Some(node_idx) = node
            && spin.si_nodes[node_idx].wn_refs > 1
        {
            spin.si_nodes[node_idx].wn_refs -= 1;
            let mut copyprev = prev;
            let mut copyp = Some(node_idx);
            while let Some(copyp_idx) = copyp {
                let np = get_wordnode(spin);
                let copyp_node = spin.si_nodes[copyp_idx];
                spin.si_nodes[np].wn_child = copyp_node.wn_child;
                if let Some(child_idx) = copyp_node.wn_child {
                    spin.si_nodes[child_idx].wn_refs += 1;
                }
                spin.si_nodes[np].wn_byte = copyp_node.wn_byte;
                if copyp_node.wn_byte == 0 {
                    spin.si_nodes[np].wn_flags = copyp_node.wn_flags;
                    spin.si_nodes[np].wn_region = copyp_node.wn_region;
                    spin.si_nodes[np].wn_affix_id = copyp_node.wn_affix_id;
                }

                spin.si_nodes[np].wn_refs = 1;
                if let Some(slot) = copyprev {
                    slot.set(&mut spin.si_nodes, Some(np));
                }
                copyprev = Some(NodeSlot::Sibling(np));

                if copyp_idx == node_idx {
                    node = Some(np);
                }
                copyp = copyp_node.wn_sibling;
            }
        }

        let wb = word_bytes.get(i).copied().unwrap_or(0);
        while let Some(node_idx) = node {
            let n = spin.si_nodes[node_idx];
            let advance = n.wn_byte < wb
                || (n.wn_byte == 0
                    && if flags < 0 {
                        (n.wn_affix_id as u32) < (affix_id as u32)
                    } else {
                        (n.wn_flags as u32) < ((flags & WN_MASK) as u32)
                            || (n.wn_flags as i32 == (flags & WN_MASK)
                                && if spin.si_sugtree != 0 {
                                    (n.wn_region as i32 & 0xffff) < region
                                } else {
                                    (n.wn_affix_id as u32) < (affix_id as u32)
                                })
                    });
            if !advance {
                break;
            }
            prev = Some(NodeSlot::Sibling(node_idx));
            node = n.wn_sibling;
        }

        let need_new = match node {
            None => true,
            Some(node_idx) => {
                let n = spin.si_nodes[node_idx];
                n.wn_byte != wb
                    || (wb == 0
                        && (flags < 0
                            || spin.si_sugtree != 0
                            || n.wn_flags as i32 != (flags & WN_MASK)
                            || n.wn_affix_id as i32 != affix_id))
            }
        };
        if need_new {
            let np = get_wordnode(spin);
            spin.si_nodes[np].wn_byte = wb;

            if let Some(node_idx) = node {
                spin.si_nodes[np].wn_refs = spin.si_nodes[node_idx].wn_refs;
                spin.si_nodes[node_idx].wn_refs = 1;
            } else {
                spin.si_nodes[np].wn_refs = 1;
            }
            if let Some(slot) = prev {
                slot.set(&mut spin.si_nodes, Some(np));
            }
            spin.si_nodes[np].wn_sibling = node;
            node = Some(np);
        }

        let node_idx = node.unwrap();
        if wb == 0 {
            spin.si_nodes[node_idx].wn_flags = flags as u16;
            spin.si_nodes[node_idx].wn_region |= region as i16;
            spin.si_nodes[node_idx].wn_affix_id = affix_id as u8;
            break;
        }
        prev = Some(NodeSlot::Child(node_idx));
        node = spin.si_nodes[node_idx].wn_child;
        i += 1;
    }

    spin.si_msg_count += 1;

    let (comp_start, comp_inc, comp_added) = spellfile_st(|st| {
        (
            st.compress_start.get(),
            st.compress_inc.get(),
            st.compress_added.get(),
        )
    });
    if spin.si_compress_cnt > 1 {
        spin.si_compress_cnt -= 1;
        if spin.si_compress_cnt == 1 {
            spin.si_blocks_cnt += comp_inc;
        }
    }

    let do_compress = if spin.si_compress_cnt == 1 {
        spin.si_free_count < MAXWLEN as i32
    } else {
        spin.si_blocks_cnt >= comp_start
    };
    if do_compress {
        spin.si_blocks_cnt -= comp_inc;
        spin.si_compress_cnt = comp_added;

        if spin.si_verbose {
            msg_start();
            msg_puts(gettext(MSG_COMPRESSING));
            msg_clr_eos();
            g_st().msg_didout.set(false);
            g_st().msg_col.set(0);
            ui_flush();
        }

        let foldroot = spin.si_foldroot;
        wordtree_compress(spin, foldroot, c"case-folded");
        if affix_id >= 0 {
            let keeproot = spin.si_keeproot;
            wordtree_compress(spin, keeproot, c"keep-case");
        }
    }

    true
}

fn get_wordnode(spin: &mut Spellinfo) -> usize {
    if let Some(n) = spin.si_first_free {
        spin.si_first_free = spin.si_nodes[n].wn_child;
        spin.si_nodes[n] = Wordnode::new();
        spin.si_free_count -= 1;
        n
    } else {
        let nodes_per_block = (SBLOCKSIZE / size_of::<Wordnode>()).max(1);
        if spin.si_nodes.len().is_multiple_of(nodes_per_block) {
            spin.si_blocks_cnt += 1;
        }
        let idx = spin.si_nodes.len();
        spin.si_nodes.push(Wordnode::new());
        idx
    }
}

fn deref_wordnode(spin: &mut Spellinfo, node: usize) -> i32 {
    let mut cnt: i32 = 0;

    spin.si_nodes[node].wn_refs -= 1;
    if spin.si_nodes[node].wn_refs == 0 {
        let mut np = Some(node);
        while let Some(np_idx) = np {
            let next = spin.si_nodes[np_idx].wn_sibling;
            if let Some(child_idx) = spin.si_nodes[np_idx].wn_child {
                cnt += deref_wordnode(spin, child_idx);
            }
            free_wordnode(spin, np_idx);
            cnt += 1;
            np = next;
        }
        cnt += 1;
    }
    cnt
}

fn free_wordnode(spin: &mut Spellinfo, n: usize) {
    spin.si_nodes[n].wn_child = spin.si_first_free;
    spin.si_first_free = Some(n);
    spin.si_free_count += 1;
}

fn wordtree_compress(spin: &mut Spellinfo, root: Option<usize>, name: &std::ffi::CStr) {
    let mut ht: std::collections::HashMap<[u8; 6], usize> = std::collections::HashMap::new();
    let mut tot: i32 = 0;

    let Some(root_idx) = root else {
        return;
    };
    let Some(first_sibling) = spin.si_nodes[root_idx].wn_sibling else {
        return;
    };

    let (n, _) = node_compress(spin, first_sibling, &mut ht, &mut tot);

    if spin.si_verbose || opt_vars().p_verbose.get() > 2 {
        let perc: i64 = if tot > 1_000_000 {
            ((tot - n) / (tot / 100)) as i64
        } else if tot == 0 {
            0
        } else {
            ((tot - n) * 100 / tot) as i64
        };
        format_to_buf!(
            &mut *g_st().iobuff_bytes_mut(),
            gettext(c"Compressed %s: %d of %d nodes; %d (%ld%%) remaining"),
            name,
            n,
            tot,
            tot - n,
            perc,
        );
        spell_message(&*spin, &g_st().iobuff_cstr());
    }
}

fn node_compress(
    spin: &mut Spellinfo,
    node: usize,
    ht: &mut std::collections::HashMap<[u8; 6], usize>,
    tot: &mut i32,
) -> (i32, [u8; 6]) {
    let mut len: i32 = 0;
    let mut compressed: i32 = 0;

    let mut np = Some(node);
    while let Some(np_idx) = np {
        if g_st().got_int.get() {
            break;
        }
        len += 1;
        if let Some(child) = spin.si_nodes[np_idx].wn_child {
            let (child_compressed, hk) = node_compress(spin, child, ht, tot);
            compressed += child_compressed;

            if let Some(&first_tp) = ht.get(&hk) {
                let mut tp = Some(first_tp);
                let mut found = false;
                while let Some(tp_idx) = tp {
                    if node_equal(&spin.si_nodes, child, tp_idx) {
                        spin.si_nodes[tp_idx].wn_refs += 1;
                        compressed += deref_wordnode(spin, child);
                        spin.si_nodes[np_idx].wn_child = Some(tp_idx);
                        found = true;
                        break;
                    }
                    tp = spin.si_nodes[tp_idx].wn_wnode;
                }
                if !found {
                    spin.si_nodes[child].wn_wnode = spin.si_nodes[first_tp].wn_wnode;
                    spin.si_nodes[first_tp].wn_wnode = Some(child);
                }
            } else {
                ht.insert(hk, child);
            }
        }
        np = spin.si_nodes[np_idx].wn_sibling;
    }
    *tot += len + 1;

    let mut hashkey = [0u8; 6];
    hashkey[0] = len as u8;
    let mut nr: u32 = 0;
    np = Some(node);
    while let Some(np_idx) = np {
        let n_node = spin.si_nodes[np_idx];
        let n: u32 = if n_node.wn_byte == 0 {
            (n_node.wn_flags as i32
                + ((n_node.wn_region as i32) << 8)
                + ((n_node.wn_affix_id as i32) << 16)) as u32
        } else {
            (n_node.wn_byte as usize).wrapping_add(n_node.wn_child.unwrap_or(0) << 8) as u32
        };
        nr = nr.wrapping_mul(101).wrapping_add(n);
        np = n_node.wn_sibling;
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

fn node_equal(nodes: &[Wordnode], n1: usize, n2: usize) -> bool {
    let mut p1 = Some(n1);
    let mut p2 = Some(n2);
    while let (Some(i1), Some(i2)) = (p1, p2) {
        let a = nodes[i1];
        let b = nodes[i2];
        if a.wn_byte != b.wn_byte
            || if a.wn_byte == 0 {
                a.wn_flags != b.wn_flags
                    || a.wn_region != b.wn_region
                    || a.wn_affix_id != b.wn_affix_id
            } else {
                a.wn_child != b.wn_child
            }
        {
            break;
        }
        p1 = a.wn_sibling;
        p2 = b.wn_sibling;
    }
    p1.is_none() && p2.is_none()
}

fn write_vim_spell(spin: &mut Spellinfo, fname_cstr: &std::ffi::CStr) -> bool {
    let mut retval = true;

    let Some(fd) = CFile::open(fname_cstr, c"w") else {
        semsg!(gettext(E_NOTOPEN), fname_cstr);
        return false;
    };

    let mut fwv = spell_write(&VIMSPELLMAGIC[..VIMSPELLMAGICL], &fd);
    if fwv != 1 {
        let _ = spell_close(fd);
        emsg(gettext(E_WRITE));
        return false;
    }

    spell_putc(VIMSPELLVERSION, &fd);

    if let Some(ref info) = spin.si_info {
        spell_putc(SN_INFO, &fd);
        spell_putc(0, &fd);
        let bytes = info.to_bytes();
        put_bytes(&fd, bytes.len() as u64, 4);
        fwv &= spell_write(bytes, &fd);
    }

    let regionmask: i32 = if spin.si_region_count > 1 {
        spell_putc(SN_REGION, &fd);
        spell_putc(SNF_REQUIRED, &fd);
        let l = (spin.si_region_count as usize) * 2;
        put_bytes(&fd, l as u64, 4);
        fwv &= spell_write(&spin.si_region_name[..l], &fd);
        (1 << spin.si_region_count) - 1
    } else {
        0
    };

    if !spin.si_ascii && spin.si_add == 0 {
        let mut folchars = [0u8; 128 * 8];

        spell_putc(SN_CHARFLAGS, &fd);
        spell_putc(SNF_REQUIRED, &fd);

        let mut l: usize = 0;
        for i in 128..256 {
            l += utf_char2bytes(
                with_spell_priv(|s| s.spelltab.get()).st_fold[i] as i32,
                &mut folchars[l..],
            ) as usize;
        }
        put_bytes(&fd, (1 + 128 + 2 + l) as u64, 4);

        spell_putc(128, &fd);
        for i in 128..256 {
            let mut flags: i32 = 0;
            if with_spell_priv(|s| s.spelltab.get()).st_isw[i] {
                flags |= CF_WORD as i32;
            }
            if with_spell_priv(|s| s.spelltab.get()).st_isu[i] {
                flags |= CF_UPPER as i32;
            }
            spell_putc(flags, &fd);
        }

        put_bytes(&fd, l as u64, 2);
        fwv &= spell_write(&folchars[..l], &fd);
    }

    if let Some(ref midword) = spin.si_midword {
        spell_putc(SN_MIDWORD, &fd);
        spell_putc(SNF_REQUIRED, &fd);

        let bytes = midword.to_bytes();
        put_bytes(&fd, bytes.len() as u64, 4);
        fwv &= spell_write(bytes, &fd);
    }

    if !spin.si_prefcond.is_empty() {
        spell_putc(SN_PREFCOND, &fd);
        spell_putc(SNF_REQUIRED, &fd);

        let l = write_spell_prefcond(None, &spin.si_prefcond, &mut fwv) as usize;
        put_bytes(&fd, l as u64, 4);

        write_spell_prefcond(Some(&fd), &spin.si_prefcond, &mut fwv);
    }

    for round in 1..=3 {
        let gap = if round == 1 {
            &mut spin.si_rep
        } else if round == 2 {
            if spin.si_sofofr.is_some() && spin.si_sofoto.is_some() {
                continue;
            }
            &mut spin.si_sal
        } else {
            &mut spin.si_repsal
        };

        if gap.is_empty() {
            continue;
        }

        if round != 2 {
            gap.sort_unstable_by(|p1, p2| p1.ft_from.as_bytes().cmp(p2.ft_from.as_bytes()));
        }

        let sect_id = if round == 1 {
            SN_REP
        } else if round == 2 {
            SN_SAL
        } else {
            SN_REPSAL
        };
        spell_putc(sect_id, &fd);
        spell_putc(0, &fd);

        let mut l: usize = 2;
        for ftp in gap.iter() {
            l += 1 + ftp.ft_from.as_bytes().len();
            l += 1 + ftp.ft_to.as_bytes().len();
        }
        if round == 2 {
            l += 1;
        }
        put_bytes(&fd, l as u64, 4);

        if round == 2 {
            let mut i: i32 = 0;
            if spin.si_followup {
                i |= SAL_F0LLOWUP;
            }
            if spin.si_collapse {
                i |= SAL_COLLAPSE;
            }
            if spin.si_rem_accents {
                i |= SAL_REM_ACCENTS;
            }
            spell_putc(i, &fd);
        }

        put_bytes(&fd, gap.len() as u64, 2);
        for ftp in gap.iter() {
            for rr in 1..=2 {
                let bytes = if rr == 1 {
                    ftp.ft_from.as_bytes()
                } else {
                    ftp.ft_to.as_bytes()
                };
                spell_putc(bytes.len() as i32, &fd);
                if !bytes.is_empty() {
                    fwv &= spell_write(bytes, &fd);
                }
            }
        }
    }

    if let (Some(sofofr), Some(sofoto)) = (&spin.si_sofofr, &spin.si_sofoto) {
        spell_putc(SN_SOFO, &fd);
        spell_putc(0, &fd);

        let b1 = sofofr.to_bytes();
        let b2 = sofoto.to_bytes();
        put_bytes(&fd, (b1.len() + b2.len() + 4) as u64, 4);

        put_bytes(&fd, b1.len() as u64, 2);
        fwv &= spell_write(b1, &fd);

        put_bytes(&fd, b2.len() as u64, 2);
        fwv &= spell_write(b2, &fd);
    }

    if !spin.si_commonwords.is_empty() {
        spell_putc(SN_WORDS, &fd);
        spell_putc(0, &fd);

        for round in 1..=2 {
            let mut len: usize = 0;
            for word in &spin.si_commonwords {
                let l = word.len() + 1;
                len += l;
                if round == 2 {
                    fwv &= spell_write(word, &fd);
                    fwv &= spell_write(&[0], &fd);
                }
            }
            if round == 1 {
                put_bytes(&fd, len as u64, 4);
            }
        }
    }

    if !spin.si_map.is_empty() {
        spell_putc(SN_MAP, &fd);
        spell_putc(0, &fd);
        put_bytes(&fd, spin.si_map.len() as u64, 4);
        fwv &= spell_write(&spin.si_map, &fd);
    }

    if spin.si_nosugfile == 0
        && (!spin.si_sal.is_empty() || (spin.si_sofofr.is_some() && spin.si_sofoto.is_some()))
    {
        spell_putc(SN_SUGFILE, &fd);
        spell_putc(0, &fd);
        put_bytes(&fd, 8, 4);

        spin.si_sugtime = os_time() as time_t;
        put_time(&fd, spin.si_sugtime);
    }

    if spin.si_nosplitsugs != 0 {
        spell_putc(SN_NOSPLITSUGS, &fd);
        spell_putc(0, &fd);
        put_bytes(&fd, 0, 4);
    }

    if spin.si_nocompoundsugs != 0 {
        spell_putc(SN_NOCOMPOUNDSUGS, &fd);
        spell_putc(0, &fd);
        put_bytes(&fd, 0, 4);
    }

    if let Some(ref compflags) = spin.si_compflags {
        spell_putc(SN_COMPOUND, &fd);
        spell_putc(0, &fd);

        let cf_bytes = compflags.to_bytes();
        let mut l = cf_bytes.len();
        for p in &spin.si_comppat {
            l += p.to_bytes().len() + 1;
        }
        put_bytes(&fd, (l + 7) as u64, 4);

        spell_putc(spin.si_compmax, &fd);
        spell_putc(spin.si_compminlen, &fd);
        spell_putc(spin.si_compsylmax, &fd);
        spell_putc(0, &fd);
        spell_putc(spin.si_compoptions, &fd);
        put_bytes(&fd, spin.si_comppat.len() as u64, 2);
        for p in &spin.si_comppat {
            let p_bytes = p.to_bytes();
            spell_putc(p_bytes.len() as i32, &fd);
            fwv &= spell_write(p_bytes, &fd);
        }
        fwv &= spell_write(cf_bytes, &fd);
    }

    if spin.si_nobreak != 0 {
        spell_putc(SN_NOBREAK, &fd);
        spell_putc(0, &fd);
        put_bytes(&fd, 0, 4);
    }

    if let Some(ref syllable) = spin.si_syllable {
        spell_putc(SN_SYLLABLE, &fd);
        spell_putc(0, &fd);

        let syl_bytes = syllable.to_bytes();
        put_bytes(&fd, syl_bytes.len() as u64, 4);
        fwv &= spell_write(syl_bytes, &fd);
    }

    spell_putc(SN_END, &fd);

    spin.si_memtot = 0;
    for round in 1..=3 {
        let root = if round == 1 {
            spin.si_foldroot
        } else if round == 2 {
            spin.si_keeproot
        } else {
            spin.si_prefroot
        };
        let tree = root.and_then(|r| (&spin.si_nodes)[r].wn_sibling);

        clear_node(&mut spin.si_nodes, tree);

        let nodecount =
            put_node(None, &mut spin.si_nodes, tree, 0, regionmask, round == 3) as usize;
        put_bytes(&fd, nodecount as u64, 4);
        spin.si_memtot += (nodecount + nodecount * size_of::<i32>()) as i32;

        put_node(
            Some(&fd),
            &mut spin.si_nodes,
            tree,
            0,
            regionmask,
            round == 3,
        );
    }

    if spell_putc(0, &fd) == EOF {
        retval = false;
    }
    if spell_close(fd) == EOF {
        retval = false;
    }

    if fwv != 1 {
        retval = false;
    }
    if !retval {
        emsg(gettext(E_WRITE));
    }

    retval
}

fn clear_node(nodes: &mut [Wordnode], node: Option<usize>) {
    let mut np = node;
    while let Some(np_idx) = np {
        nodes[np_idx].wn_index = 0;
        nodes[np_idx].wn_wnode = None;

        if nodes[np_idx].wn_byte != 0 {
            let child = nodes[np_idx].wn_child;
            clear_node(nodes, child);
        }
        np = nodes[np_idx].wn_sibling;
    }
}

fn put_node(
    fd: Option<&CFile>,
    nodes: &mut [Wordnode],
    node: Option<usize>,
    idx: i32,
    regionmask: i32,
    prefixtree: bool,
) -> i32 {
    let Some(node_idx) = node else {
        return 0;
    };

    nodes[node_idx].wn_index = idx;

    let mut siblingcount: i32 = 0;
    let mut np = Some(node_idx);
    while let Some(np_idx) = np {
        siblingcount += 1;
        np = nodes[np_idx].wn_sibling;
    }

    if let Some(fd) = fd {
        spell_putc(siblingcount, fd);
    }

    np = Some(node_idx);
    while let Some(np_idx) = np {
        let n = nodes[np_idx];
        if n.wn_byte == 0 {
            if let Some(fd) = fd {
                if prefixtree {
                    if n.wn_flags == PFX_FLAGS as u16 {
                        spell_putc(BY_NOFLAGS, fd);
                    } else {
                        spell_putc(BY_FLAGS, fd);
                        spell_putc(n.wn_flags as i32, fd);
                    }
                    spell_putc(n.wn_affix_id as i32, fd);
                    put_bytes(fd, n.wn_region as u16 as u64, 2);
                } else {
                    let mut flags = n.wn_flags as i32;
                    if regionmask != 0 && (n.wn_region as i32) != regionmask {
                        flags |= WF_REGION;
                    }
                    if n.wn_affix_id != 0 {
                        flags |= WF_AFX;
                    }
                    if flags == 0 {
                        spell_putc(BY_NOFLAGS, fd);
                    } else {
                        if n.wn_flags >= 0x100 {
                            spell_putc(BY_FLAGS2, fd);
                            spell_putc(flags, fd);
                            spell_putc(((flags as u32) >> 8) as i32, fd);
                        } else {
                            spell_putc(BY_FLAGS, fd);
                            spell_putc(flags, fd);
                        }
                        if (flags & WF_REGION) != 0 {
                            spell_putc(n.wn_region as i32, fd);
                        }
                        if (flags & WF_AFX) != 0 {
                            spell_putc(n.wn_affix_id as i32, fd);
                        }
                    }
                }
            }
        } else {
            let child = n.wn_child.unwrap();
            if nodes[child].wn_index != 0 && nodes[child].wn_wnode != Some(node_idx) {
                if let Some(fd) = fd {
                    spell_putc(BY_INDEX, fd);
                    put_bytes(fd, nodes[child].wn_index as u64, 3);
                }
            } else if nodes[child].wn_wnode.is_none() {
                nodes[child].wn_wnode = Some(node_idx);
            }

            if let Some(fd) = fd
                && spell_putc(n.wn_byte as i32, fd) == EOF
            {
                emsg(gettext(E_WRITE));
                return 0;
            }
        }
        np = n.wn_sibling;
    }

    let mut newindex = idx + siblingcount + 1;

    np = Some(node_idx);
    while let Some(np_idx) = np {
        let n = nodes[np_idx];
        if n.wn_byte != 0 {
            let child = n.wn_child.unwrap();
            if nodes[child].wn_wnode == Some(node_idx) {
                newindex = put_node(fd, nodes, Some(child), newindex, regionmask, prefixtree);
            }
        }
        np = n.wn_sibling;
    }

    newindex
}

/// ":mkspell [-ascii] outfile  infile ..."
/// ":mkspell [-ascii] addfile"
pub fn ex_mkspell(eap: &mut Exarg) {
    let mut fnames: Vec<std::ffi::CString> = Vec::new();
    let mut arg = eap.arg.clone();
    let mut ascii = false;

    if (arg).as_bytes().starts_with(b"-ascii") {
        ascii = true;
        arg = arg.wrapping_add(6).skipwhite();
    }

    if !get_arglist_exp((arg).as_bytes(), &mut fnames, false) {
        return;
    }

    mkspell(&fnames, ascii, eap.forceit, false);
}

fn spell_make_sugfile(spin: &mut Spellinfo, wfname_cstr: &std::ffi::CStr) {
    let found_idx = with_spell_ext(|ext| {
        ext.first_lang
            .borrow()
            .iter()
            .enumerate()
            .rev()
            .find(|(_, s)| {
                s.as_ref().is_some_and(|s| {
                    s.sl_fname
                        .as_ref()
                        .is_some_and(|f| path_equal(wfname_cstr, f, K_PATH_CMP_EXPAND))
                })
            })
            .map(|(idx, _)| idx)
    });
    let mut slang_box: Box<Slang> = if let Some(idx) = found_idx {
        let rc = with_spell_ext(|ext| ext.first_lang.borrow_mut()[idx].take().unwrap());
        Box::new(std::rc::Rc::try_unwrap(rc).unwrap_or_else(|_| unreachable!()))
    } else {
        spell_message(&*spin, gettext(c"Reading back spell file..."));
        let mut new_lp = slang_alloc(None::<&std::ffi::CStr>);
        new_lp.sl_fname = Some(wfname_cstr.to_owned());
        new_lp.sl_add = path_tail(wfname_cstr)
            .to_bytes()
            .windows(SPL_FNAME_ADD.len())
            .any(|w| w == SPL_FNAME_ADD);
        if !spell_load_file_into(wfname_cstr, &mut new_lp, false) {
            return;
        }
        new_lp
    };

    spin.si_nodes.clear();
    spin.si_blocks_cnt = 0;
    spin.si_compress_cnt = 0;
    spin.si_free_count = 0;
    spin.si_first_free = None;
    spin.si_foldwcount = 0;

    spell_message(&*spin, gettext(c"Performing soundfolding..."));
    if sug_filltree(spin, &mut slang_box) && sug_maketable(spin) {
        smsg!(
            0,
            gettext(c"Number of words after soundfolding: %lld"),
            spin.si_spellbuf.as_ref().unwrap().b_ml.ml_line_count.get() as i64,
        );

        spell_message(&*spin, gettext(MSG_COMPRESSING));
        wordtree_compress(spin, spin.si_foldroot, c"case-folded");

        let mut fname_buf = [0u8; MAXPATHL];
        crate::memory::copy_bytes_to_c_buf(&mut fname_buf, wfname_cstr.to_bytes());
        let len = crate::memory::bytes_until_nul(&fname_buf).len();
        fname_buf[len - 2] = b'u';
        fname_buf[len - 1] = b'g';
        sug_write(spin, crate::memory::bytes_as_cstr(&fname_buf));
    }

    if let Some(idx) = found_idx {
        with_spell_ext(|ext| {
            ext.first_lang.borrow_mut()[idx] = Some(std::rc::Rc::from(slang_box));
        });
    } else {
        drop(slang_box);
    }
    spin.si_nodes.clear();
    close_spellbuf(&mut spin.si_spellbuf);
}

fn sug_filltree(spin: &mut Spellinfo, slang: &mut Slang) -> bool {
    let mut arridx = [0 as Idx; MAXWLEN];
    let mut curi = [0_i32; MAXWLEN];
    let mut tword = [0u8; MAXWLEN];
    let mut tsalword = [0u8; MAXWLEN];
    let mut words_done: u32 = 0;
    let mut wordcount = [0_i32; MAXWLEN];

    spin.si_foldroot = wordtree_alloc(&mut *spin);
    spin.si_sugtree = 1;

    if slang.sl_fbyts.is_empty() || slang.sl_fidxs.is_empty() {
        return false;
    }
    let fbyts_len = slang.sl_fbyts.len() as i32;

    arridx[0] = 0;
    curi[0] = 1;
    wordcount[0] = 0;

    let mut depth: isize = 0;
    while depth >= 0 && !g_st().got_int.get() {
        let d = depth as usize;
        if curi[d] > i32::from(slang.sl_fbyts[arridx[d] as usize]) {
            slang.sl_fidxs[arridx[d] as usize] = wordcount[d];
            if depth > 0 {
                wordcount[d - 1] += wordcount[d];
            }
            depth -= 1;
            line_breakcheck();
        } else {
            let mut n = arridx[d] + curi[d];
            curi[d] += 1;

            let c = i32::from(slang.sl_fbyts[n as usize]);
            if c == 0 {
                tword[d] = 0;
                spell_soundfold(slang, &tword[..d], true, &mut tsalword);

                if !tree_add_word(
                    spin,
                    &tsalword,
                    spin.si_foldroot,
                    (words_done >> 16) as i32,
                    (words_done & 0xffff) as i32,
                    0,
                ) {
                    return false;
                }

                words_done += 1;
                wordcount[d] += 1;
                spin.si_blocks_cnt = 0;

                while n + 1 < fbyts_len && slang.sl_fbyts[(n + 1) as usize] == 0 {
                    n += 1;
                    curi[d] += 1;
                }
            } else if d < MAXWLEN - 1 {
                tword[d] = c as u8;
                depth += 1;
                let nd = depth as usize;
                arridx[nd] = slang.sl_fidxs[n as usize];
                curi[nd] = 1;
                wordcount[nd] = 0;
            }
        }
    }

    smsg!(0, gettext(c"Total number of words: %d"), words_done,);
    true
}

fn sug_maketable(spin: &mut Spellinfo) -> bool {
    let mut ga: Vec<u8> = Vec::with_capacity(100);
    let mut res = true;

    spin.si_spellbuf = Some(open_spellbuf());

    let first_sibling = spin.si_foldroot.and_then(|r| spin.si_nodes[r].wn_sibling);
    if sug_filltable(spin, first_sibling, 0, &mut ga) == -1 {
        res = false;
    }

    res
}

fn sug_filltable(
    spin: &mut Spellinfo,
    node: Option<usize>,
    startwordnr: i32,
    gap: &mut Vec<u8>,
) -> i32 {
    let mut wordnr = startwordnr;

    let mut p = node;
    while let Some(p_idx) = p {
        if spin.si_nodes[p_idx].wn_byte == 0 {
            gap.clear();
            let mut prev_nr: i32 = 0;
            let mut np = Some(p_idx);
            while let Some(np_idx) = np {
                let n = spin.si_nodes[np_idx];
                if n.wn_byte != 0 {
                    break;
                }
                let mut nr = ((n.wn_flags as i32) << 16) + (n.wn_region as i32 & 0xffff);
                nr -= prev_nr;
                prev_nr += nr;
                let mut obuf = [0u8; 8];
                let olen = offset2bytes(nr, &mut obuf) as usize;
                gap.extend_from_slice(&obuf[..olen]);
                np = n.wn_sibling;
            }

            gap.push(0);

            let spellbuf = spin.si_spellbuf.as_deref().unwrap();
            let text_len = gap.len().saturating_sub(1);
            if !ml_append_buf(
                spellbuf,
                wordnr as Linenr,
                &gap[..text_len],
                gap.len() as i32,
                true,
            ) {
                return -1;
            }
            wordnr += 1;

            while let Some(sib_idx) = spin.si_nodes[p_idx].wn_sibling {
                if spin.si_nodes[sib_idx].wn_byte != 0 {
                    break;
                }
                spin.si_nodes[p_idx].wn_sibling = spin.si_nodes[sib_idx].wn_sibling;
            }

            spin.si_nodes[p_idx].wn_flags = 0;
            spin.si_nodes[p_idx].wn_region = 0;
        } else {
            let child = spin.si_nodes[p_idx].wn_child;
            wordnr = sug_filltable(spin, child, wordnr, gap);
            if wordnr == -1 {
                return -1;
            }
        }
        p = spin.si_nodes[p_idx].wn_sibling;
    }
    wordnr
}

fn offset2bytes(nr: i32, buf: &mut [u8; 8]) -> i32 {
    let b1 = nr % 255 + 1;
    let mut rem = nr / 255;
    let b2 = rem % 255 + 1;
    rem /= 255;
    let b3 = rem % 255 + 1;
    let b4 = rem / 255 + 1;

    if b4 > 1 || b3 > 0x1f {
        buf[0] = (0xe0 + b4) as u8;
        buf[1] = b3 as u8;
        buf[2] = b2 as u8;
        buf[3] = b1 as u8;
        return 4;
    }
    if b3 > 1 || b2 > 0x3f {
        buf[0] = (0xc0 + b3) as u8;
        buf[1] = b2 as u8;
        buf[2] = b1 as u8;
        return 3;
    }
    if b2 > 1 || b1 > 0x7f {
        buf[0] = (0x80 + b2) as u8;
        buf[1] = b1 as u8;
        return 2;
    }
    buf[0] = b1 as u8;
    1
}

fn sug_write(spin: &mut Spellinfo, fname_cstr: &std::ffi::CStr) {
    let Some(fd) = CFile::open(fname_cstr, c"w") else {
        semsg!(gettext(E_NOTOPEN), fname_cstr);
        return;
    };

    format_to_buf!(
        &mut *g_st().iobuff_bytes_mut(),
        gettext(c"Writing suggestion file %s..."),
        fname_cstr,
    );
    spell_message(&*spin, &g_st().iobuff_cstr());

    if spell_write(&VIMSUGMAGIC[..VIMSUGMAGICL], &fd) != 1 {
        emsg(gettext(E_WRITE));
        spell_close(fd);
        return;
    }
    spell_putc(VIMSUGVERSION, &fd);
    put_time(&fd, spin.si_sugtime);

    spin.si_memtot = 0;
    let tree = spin
        .si_foldroot
        .and_then(|r| (&spin.si_nodes)[r].wn_sibling);
    clear_node(&mut spin.si_nodes, tree);

    let nodecount = put_node(None, &mut spin.si_nodes, tree, 0, 0, false) as usize;
    put_bytes(&fd, nodecount as u64, 4);
    spin.si_memtot += (nodecount + nodecount * size_of::<i32>()) as i32;

    put_node(Some(&fd), &mut spin.si_nodes, tree, 0, 0, false);

    let spellbuf = spin.si_spellbuf.as_deref().unwrap();
    let wcount = spellbuf.b_ml.ml_line_count.get();
    put_bytes(&fd, wcount as u64, 4);

    let mut write_err = false;
    let mut memtot_add = 0;
    for lnum in 1..=wcount {
        let line_slice = ml_get_buf(spellbuf, lnum).to_bytes_with_nul();
        let len = line_slice.len() as i32;
        if spell_write(line_slice, &fd) == 0 {
            emsg(gettext(E_WRITE));
            write_err = true;
            break;
        }
        memtot_add += len;
    }
    spin.si_memtot += memtot_add;

    if !write_err {
        if spell_putc(0, &fd) == EOF {
            emsg(gettext(E_WRITE));
        }

        format_to_buf!(
            &mut *g_st().iobuff_bytes_mut(),
            gettext(c"Estimated runtime memory use: %d bytes"),
            spin.si_memtot,
        );
        spell_message(&*spin, &g_st().iobuff_cstr());
    }

    spell_close(fd);
}

fn mkspell(fnames: &[std::ffi::CString], ascii: bool, over_write: bool, added_word: bool) {
    let mut fname_buf = [0u8; MAXPATHL];
    let mut wfname_buf = [0u8; MAXPATHL];
    let mut afile: [Option<Afffile>; MAXREGIONS] = [const { None }; MAXREGIONS];
    let mut error = false;
    let mut spin = Spellinfo::default();

    spell_check_msm(opt_vars().p_msm.as_bytes(), true);

    spin.si_verbose = !added_word;
    spin.si_ascii = ascii;
    spin.si_followup = true;
    spin.si_rem_accents = true;
    spin.si_newcomp_id = 127;

    let fcount = fnames.len() as i32;
    let innames = if fcount == 1 { fnames } else { &fnames[1..] };
    let mut incount = fcount - 1;

    if let Some(f0_cstr) = fnames.first() {
        let f0_bytes = f0_cstr.as_bytes();
        let len = f0_bytes.len();
        if fcount == 1 && len > 4 && &f0_bytes[len - 4..] == b".add" {
            incount = 1;
            format_to_buf!(&mut wfname_buf[..], c"%s.spl", f0_cstr);
        } else if fcount == 1 {
            incount = 1;
            let sp_enc = spell_enc();
            format_to_buf!(
                &mut wfname_buf[..],
                SPL_FNAME_TMPL,
                f0_cstr,
                if spin.si_ascii { c"ascii" } else { &sp_enc },
            );
        } else if len > 4 && &f0_bytes[len - 4..] == b".spl" {
            crate::memory::copy_bytes_to_c_buf(&mut wfname_buf, f0_bytes);
        } else {
            let sp_enc = spell_enc();
            format_to_buf!(
                &mut wfname_buf[..],
                SPL_FNAME_TMPL,
                f0_cstr,
                if spin.si_ascii { c"ascii" } else { &sp_enc },
            );
        }

        let wfname = crate::memory::bytes_as_cstr(&wfname_buf);

        if path_tail(wfname)
            .to_bytes()
            .windows(SPL_FNAME_ASCII.len())
            .any(|w| w == SPL_FNAME_ASCII)
        {
            spin.si_ascii = true;
        }
        if path_tail(wfname)
            .to_bytes()
            .windows(SPL_FNAME_ADD.len())
            .any(|w| w == SPL_FNAME_ADD)
        {
            spin.si_add = 1;
        }
    }

    let wfname_cstr = crate::memory::bytes_as_cstr(&wfname_buf);

    if incount <= 0 {
        emsg(gettext(E_INVARG));
    } else if vim_strchr(path_tail(wfname_cstr), b'_' as i32).is_some() {
        emsg(gettext(c"E751: Output file name must not have region name"));
    } else if incount > MAXREGIONS as i32 {
        semsg!(
            gettext(c"E754: Only up to %d regions supported"),
            MAXREGIONS as i32,
        );
    } else if !over_write && os_path_exists(wfname_cstr) {
        emsg(gettext(E_EXISTS));
    } else if os_isdir(wfname_cstr) {
        semsg!(gettext(E_ISADIR2), wfname_cstr);
    } else {
        let mut region_ok = true;
        for (i, slot) in afile.iter_mut().enumerate().take(incount as usize) {
            *slot = None;

            if incount > 1 {
                let in_cstr = &innames[i];
                let in_bytes = in_cstr.as_bytes();
                let len = in_bytes.len();
                if path_tail(in_cstr).to_bytes().len() < 5 || in_bytes[len - 3] != b'_' {
                    semsg!(gettext(c"E755: Invalid region in %s"), in_cstr);
                    region_ok = false;
                    break;
                }
                spin.si_region_name[i * 2] = in_bytes[len - 2].to_ascii_lowercase();
                spin.si_region_name[i * 2 + 1] = in_bytes[len - 1].to_ascii_lowercase();
            }
        }

        if region_ok {
            spin.si_region_count = incount;
            spin.si_foldroot = wordtree_alloc(&mut spin);
            spin.si_keeproot = wordtree_alloc(&mut spin);
            spin.si_prefroot = wordtree_alloc(&mut spin);

            if spin.si_add == 0 {
                spin.si_clear_chartab = 1;
            }

            let mut i: usize = 0;
            while (i as i32) < incount && !error {
                let in_cstr = &innames[i];
                spin.si_conv.vc_type = CONV_NONE;
                spin.si_region = 1 << i;

                format_to_buf!(&mut fname_buf[..], c"%s.aff", in_cstr);
                if os_path_exists(crate::memory::bytes_as_cstr(&fname_buf)) {
                    afile[i] = spell_read_aff(&mut spin, crate::memory::bytes_as_cstr(&fname_buf));
                    if let Some(ref aff) = afile[i] {
                        format_to_buf!(&mut fname_buf[..], c"%s.dic", in_cstr);
                        if !spell_read_dic(&mut spin, crate::memory::bytes_as_cstr(&fname_buf), aff)
                        {
                            error = true;
                        }
                    } else {
                        error = true;
                    }
                } else if !spell_read_wordfile(&mut spin, in_cstr) {
                    error = true;
                }

                convert_setup(&mut spin.si_conv, None, None);
                i += 1;
            }

            if spin.si_compflags.is_some() && spin.si_nobreak != 0 {
                msg(
                    gettext(c"Warning: both compounding and NOBREAK specified"),
                    0,
                );
            }

            if !error && !g_st().got_int.get() {
                spell_message(&spin, gettext(MSG_COMPRESSING));
                let foldroot = spin.si_foldroot;
                wordtree_compress(&mut spin, foldroot, c"case-folded");
                let keeproot = spin.si_keeproot;
                wordtree_compress(&mut spin, keeproot, c"keep-case");
                let prefroot = spin.si_prefroot;
                wordtree_compress(&mut spin, prefroot, c"prefixes");
            }

            if !error && !g_st().got_int.get() {
                format_to_buf!(
                    &mut *g_st().iobuff_bytes_mut(),
                    gettext(c"Writing spell file %s..."),
                    wfname_cstr,
                );
                spell_message(&spin, &g_st().iobuff_cstr());

                error = !write_vim_spell(&mut spin, crate::memory::bytes_as_cstr(&wfname_buf));

                spell_message(&spin, gettext(c"Done!"));
                format_to_buf!(
                    &mut *g_st().iobuff_bytes_mut(),
                    gettext(c"Estimated runtime memory use: %d bytes"),
                    spin.si_memtot,
                );
                spell_message(&spin, &g_st().iobuff_cstr());

                if !error {
                    spell_reload_one(crate::memory::bytes_as_cstr(&wfname_buf), added_word);
                }
            }

            spin.si_commonwords.clear();

            for slot in afile.iter_mut().take(incount as usize) {
                *slot = None;
            }

            spin.si_nodes.clear();

            if spin.si_sugtime != 0 && !error && !g_st().got_int.get() {
                spell_make_sugfile(&mut spin, crate::memory::bytes_as_cstr(&wfname_buf));
            }
        }
    }
}

fn spell_message(spin: &Spellinfo, str_: &std::ffi::CStr) {
    if spin.si_verbose || opt_vars().p_verbose.get() > 2 {
        if !spin.si_verbose {
            verbose_enter();
        }
        msg(str_, 0);
        ui_flush();
        if !spin.si_verbose {
            verbose_leave();
        }
    }
}

/// ":[count]spellgood  {word}"
/// ":[count]spellwrong {word}"
/// ":[count]spellundo  {word}"
/// ":[count]spellrare  {word}"
pub fn ex_spell(eap: &mut Exarg) {
    let what = if eap.cmdidx == Cmdidx::CmdSpellwrong {
        SPELL_ADD_BAD
    } else if eap.cmdidx == Cmdidx::CmdSpellrare {
        SPELL_ADD_RARE
    } else {
        SPELL_ADD_GOOD
    };
    spell_add_word(
        eap.arg.as_bytes(),
        what,
        if eap.forceit { 0 } else { eap.line2 },
        eap.cmdidx == Cmdidx::CmdSpellundo,
    );
}

/// Add `word_bytes` to 'spellfile' as a good or bad word.
pub fn spell_add_word(word_bytes: &[u8], what: SpellAddType, idx: i32, undo: bool) {
    let mut buf = crate::globals::BufPtr::NULL;
    let mut new_spf = false;
    let mut fnamebuf_arr = [0u8; MAXPATHL];
    let mut line = [0u8; MAXWLEN * 2];

    if !valid_spell_word(word_bytes) {
        emsg(gettext(E_ILLEGAL_CHARACTER_IN_WORD));
        return;
    }

    let int_wl_owned: Option<std::ffi::CString>;
    let fname_cstr: &CStr = if idx == 0 {
        int_wl_owned = with_spell_ext(|ext| {
            let mut wl = ext.int_wordlist.borrow_mut();
            if wl.is_none() {
                *wl = vim_tempname();
            }
            wl.clone()
        });
        let Some(ref wl) = int_wl_owned else {
            return;
        };
        wl.as_c_str()
    } else {
        if win_s(&curwin_mut()).b_p_spf.is_empty() {
            init_spellfile();
            new_spf = true;
        }

        if win_s(&curwin_mut()).b_p_spf.is_empty() {
            semsg!(gettext(E_NOTSET), c"spellfile");
            return;
        }

        let mut spf = win_s(&curwin_mut()).b_p_spf.as_bytes();
        let mut i: i32 = 1;
        while !spf.is_empty() {
            copy_option_part(&mut spf, &mut fnamebuf_arr[..MAXPATHL], c",");
            if i == idx {
                break;
            }
            if spf.is_empty() {
                semsg!(gettext(c"E765: 'spellfile' does not have %d entries"), idx,);
                return;
            }
            i += 1;
        }

        let fnamebuf_cstr = crate::memory::bytes_as_cstr(&fnamebuf_arr);
        buf = crate::globals::BufPtr::from(buflist_findname_exp(Some(fnamebuf_cstr)));
        if !buf.is_null() && buf.b_ml.ml_mfp.borrow().is_none() {
            buf = crate::globals::BufPtr::NULL;
        }
        if !buf.is_null() && buf_is_changed_fn(buf.borrow_mut()) {
            emsg(gettext(E_BUFLOADED));
            return;
        }

        fnamebuf_cstr
    };

    let fname_opt = Some(fname_cstr);
    let mut did_open = false;
    if what == SPELL_ADD_BAD || undo {
        let mut fpos_next: i64 = 0;
        let mut fd = CFile::open(fname_cstr, c"r");
        did_open = fd.is_some();
        let ulen = word_bytes.len();
        while let Some(ref cur_fd) = fd
            && !vim_fgets(&mut line, cur_fd)
        {
            let fpos = fpos_next;
            fpos_next = cur_fd.tell();
            if fpos_next < 0 {
                break;
            }
            let linelen = crate::memory::bytes_until_nul(&line).len();
            if linelen >= ulen
                && word_bytes == &line[..ulen]
                && (line[ulen] == b'/' || line[ulen] < b' ')
            {
                drop(fd.take());
                fd = CFile::open(fname_cstr, c"r+");
                did_open = fd.is_some();
                let Some(ref rw_fd) = fd else {
                    break;
                };
                if rw_fd.seek(fpos, SEEK_SET) == 0 {
                    rw_fd.putc(b'#' as i32);
                    if undo {
                        home_replace(None, fname_opt, &mut *g_st().name_buff_bytes_mut(), true);
                        let word_cstr = crate::memory::bytes_to_cstring(word_bytes);
                        smsg!(
                            0,
                            gettext(c"Word '%.*s' removed from %s"),
                            ulen as i32,
                            &word_cstr,
                            &*g_st().name_buff.borrow(),
                        );
                    }
                }
                if rw_fd.seek(fpos_next, SEEK_SET) != 0 {
                    let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);

                    semsg!(
                        c"%s: %s",
                        gettext(c"Seek error in spellfile"),
                        strerror(err),
                    );

                    break;
                }
            }
        }
    }

    if !undo {
        let mut fd = CFile::open(fname_cstr, c"a");
        if fd.is_none() && new_spf && !dir_of_file_exists(fname_cstr) {
            let p = path_tail_with_sep(fname_cstr).as_ptr();
            if p != fname_cstr.as_ptr() {
                let dir_len = (p as usize) - (fname_cstr.as_ptr() as usize);
                let dir_cstr = crate::memory::bytes_to_cstring(&fname_cstr.to_bytes()[..dir_len]);
                os_mkdir(&dir_cstr, 0o755);
                fd = CFile::open(fname_cstr, c"a");
            }
        }

        did_open = fd.is_some();
        if let Some(fd) = fd {
            fd.write_all(word_bytes);
            if what == SPELL_ADD_BAD {
                fd.write_all(b"/!\n");
            } else if what == SPELL_ADD_RARE {
                fd.write_all(b"/?\n");
            } else {
                fd.putc(b'\n' as i32);
            }
            drop(fd);

            home_replace(None, fname_opt, &mut *g_st().name_buff_bytes_mut(), true);
            let word_cstr = crate::memory::bytes_to_cstring(word_bytes);
            smsg!(
                0,
                gettext(c"Word '%.*s' added to %s"),
                word_bytes.len() as i32,
                &word_cstr,
                &*g_st().name_buff.borrow(),
            );
        } else {
            semsg!(gettext(E_NOTOPEN), fname_cstr);
        }
    }

    if did_open {
        mkspell(&[fname_cstr.to_owned()], false, true, true);

        if !buf.is_null() {
            let orig_mode = buf.b_orig_mode.get();
            buf_reload(buf.borrow_mut(), orig_mode, false);
        }

        redraw_all_later(UPD_SOME_VALID);
    }
}

fn init_spellfile() {
    let mut aspath = false;
    let mut lstart = curbuf_mut().b_s.b_p_spl.as_cstr();

    if win_s(&curwin_mut()).b_p_spl.is_empty() || win_s(&curwin_mut()).b_langp.borrow().is_empty() {
        return;
    }

    let spl_cstr = win_s(&curwin_mut()).b_p_spl.as_cstr();
    let spl_bytes = spl_cstr.to_bytes();
    let mut lend_off = 0usize;
    while lend_off < spl_bytes.len() && vim_strchr(c",._", i32::from(spl_bytes[lend_off])).is_none()
    {
        if vim_ispathsep(spl_bytes[lend_off]) {
            aspath = true;
            lstart = &spl_cstr[lend_off + 1..];
        }
        lend_off += 1;
    }

    let mut buf_arr = [0u8; MAXPATHL];
    let buf_len = MAXPATHL;

    if !aspath {
        let xdg_path = get_xdg_home(XDGVarType::XDGDataHome).unwrap_or_default();
        crate::memory::copy_bytes_to_c_buf(&mut buf_arr, xdg_path.as_bytes());
        crate::memory::append_bytes_to_c_buf(&mut buf_arr, b"/site/spell");

        let mut failed_dir: Option<std::ffi::CString> = None;
        if os_mkdir_recurse(
            crate::memory::bytes_as_cstr(&buf_arr),
            0o755,
            &mut failed_dir,
            None,
        ) != 0
        {
            return;
        }
    } else {
        let plen = (spl_cstr.as_ptr().wrapping_add(lend_off) as usize)
            - (curbuf_mut().b_s.b_p_spl.as_ptr() as usize);
        if plen >= buf_len {
            return;
        }
        buf_arr[..plen].copy_from_slice(&curbuf_mut().b_s.b_p_spl.as_bytes()[..plen]);
        buf_arr[plen] = 0;
    }

    let cur_len = crate::memory::bytes_as_cstr(&buf_arr).to_bytes().len();
    let lstart_len =
        ((spl_cstr.as_ptr().wrapping_add(lend_off) as isize) - (lstart.as_ptr() as isize)) as i32;
    format_to_buf!(&mut buf_arr[cur_len..], c"/%.*s", lstart_len, lstart);

    let slang0 = win_s(&curwin_mut()).b_langp.borrow()[0].slang();
    let fname_opt = slang0.sl_fname.as_deref();
    let sp_enc = spell_enc();
    let enc_suffix =
        if fname_opt.is_some_and(|f| path_tail(f).to_bytes().windows(7).any(|w| w == b".ascii.")) {
            c"ascii"
        } else {
            &sp_enc
        };
    let cur_len2 = crate::memory::bytes_as_cstr(&buf_arr).to_bytes().len();
    format_to_buf!(&mut buf_arr[cur_len2..], c".%s.add", enc_suffix);

    let buf_cstr = crate::memory::bytes_as_cstr(&buf_arr);
    set_option_value_give_err(K_OPT_SPELLFILE, cstr_as_obj(Some(buf_cstr)), OPT_LOCAL);
}

fn set_spell_charflags(flags_in: &[u8], cnt: i32, fol: &[u8]) {
    let flags = &flags_in[..(cnt.max(0) as usize).min(flags_in.len())];
    let mut new_st = Spelltab::new();

    clear_spell_chartab(&mut new_st);
    let mut fold_rem = crate::memory::bytes_until_nul(fol);

    for i in 0..128 {
        if let Some(&fl) = flags.get(i) {
            new_st.st_isw[i + 128] = (fl & CF_WORD) != 0;
            new_st.st_isu[i + 128] = (fl & CF_UPPER) != 0;
        }

        if !fold_rem.is_empty() {
            let c = mb_ptr2char_adv(&mut fold_rem);
            new_st.st_fold[i + 128] = c as u8;
            if (i + 128) as i32 != c && new_st.st_isu[i + 128] && c < 256 {
                new_st.st_upper[c as usize] = (i + 128) as u8;
            }
        }
    }

    set_spell_finish(&mut new_st);
}

fn set_spell_finish(new_st: &mut Spelltab) -> bool {
    if with_spell_priv(|s| s.did_set_spelltab.get()) {
        for i in 0..256 {
            if with_spell_priv(|s| s.spelltab.get()).st_isw[i] != new_st.st_isw[i]
                || with_spell_priv(|s| s.spelltab.get()).st_isu[i] != new_st.st_isu[i]
                || with_spell_priv(|s| s.spelltab.get()).st_fold[i] != new_st.st_fold[i]
                || with_spell_priv(|s| s.spelltab.get()).st_upper[i] != new_st.st_upper[i]
            {
                emsg(gettext(c"E763: Word characters differ between spell files"));
                return false;
            }
        }
    } else {
        with_spell_priv(|s| {
            s.spelltab.set(*new_st);
            s.did_set_spelltab.set(true);
        });
    }

    true
}

fn write_spell_prefcond(
    fd: Option<&CFile>,
    gap: &[Option<std::ffi::CString>],
    fwv: &mut usize,
) -> i32 {
    if let Some(fd) = fd {
        put_bytes(fd, gap.len() as u64, 2);
    }
    let mut totlen = 2 + gap.len();
    for p in gap {
        if let Some(s) = p {
            let bytes = s.to_bytes();
            if let Some(fd) = fd {
                spell_putc(bytes.len() as i32, fd);
                *fwv &= spell_write(bytes, fd);
            }
            totlen += bytes.len();
        } else if let Some(fd) = fd {
            spell_putc(0, fd);
        }
    }

    totlen as i32
}

fn set_map_str(lp: &mut Slang, map: &[u8]) {
    let mut headc: i32 = 0;
    let map_bytes = crate::memory::bytes_until_nul(map);

    if map_bytes.is_empty() {
        lp.sl_has_map = false;
        return;
    }
    lp.sl_has_map = true;

    for i in 0..256 {
        lp.sl_map_array[i] = 0;
    }
    lp.sl_map_hash.clear();

    let mut rem = map_bytes;
    while !rem.is_empty() {
        let c = mb_cptr2char_adv(&mut rem);
        if c == b'/' as i32 {
            headc = 0;
        } else {
            if headc == 0 {
                headc = c;
            }

            if c >= 256 {
                if lp.sl_map_hash.insert(c, headc).is_some() {
                    emsg(gettext(E_DUPLICATE_CHAR_IN_MAP_ENTRY));
                }
            } else {
                lp.sl_map_array[c as usize] = headc;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset2bytes_and_spell_check_msm_work() {
        let mut buf = [0u8; 8];
        assert_eq!(offset2bytes(0, &mut buf), 1);
        assert_eq!(buf[0], 1);

        assert_eq!(offset2bytes(254, &mut buf), 2);
        assert_eq!(buf[0], 0x81);
        assert_eq!(buf[1], 255);

        assert!(spell_check_msm(b"460000,2000,500", false));
        assert!(!spell_check_msm(b"0,2000,500", false));
    }
}
