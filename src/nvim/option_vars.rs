//! Global variables for settable options (`src/nvim/option_vars.h`).
#![forbid(unsafe_code)]

use crate::eval::typval::Callback;
use crate::option::OptInt;
use crate::optionstr::{OptIndex, OptSchemaItem, OptSchemaKind};
use crate::regexp::Regprog;
use crate::syntax::{IdlCache, Keyentry, SynCluster, SynTime, Synpat, Synstate};
use crate::window::K_WIN_OPT_COUNT;

thread_local! {
    static INTERNED_OPT_CSTRS: std::cell::RefCell<
        std::collections::HashSet<&'static std::ffi::CStr>,
    > = std::cell::RefCell::new(std::collections::HashSet::new());
}

#[inline]
pub(crate) fn intern_opt_cstr(s: &std::ffi::CStr) -> &'static std::ffi::CStr {
    if s.is_empty() {
        return c"";
    }
    INTERNED_OPT_CSTRS.with(|set| {
        let mut set = set.borrow_mut();
        if let Some(&existing) = set.get(s) {
            return existing;
        }
        let leaked: &'static std::ffi::CStr = Box::leak(s.to_owned().into_boxed_c_str());
        set.insert(leaked);
        leaked
    })
}

#[repr(C)]
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct OptString(std::cell::Cell<Option<&'static std::ffi::CStr>>);

impl OptString {
    pub const fn new() -> Self {
        Self(std::cell::Cell::new(None))
    }

    #[inline(always)]
    pub fn from_cstr(s: &std::ffi::CStr) -> Self {
        Self(std::cell::Cell::new(Some(intern_opt_cstr(s))))
    }

    #[inline(always)]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let bytes = crate::memory::bytes_until_nul(bytes);
        Self::from_cstr(&crate::memory::bytes_to_cstring(bytes))
    }

    #[inline(always)]
    pub fn set(&self, other: Self) {
        self.0.set(other.0.get());
    }

    #[inline(always)]
    pub fn replace(&self, other: Self) -> Self {
        Self(std::cell::Cell::new(self.0.replace(other.0.get())))
    }

    #[inline(always)]
    pub fn take(&self) -> Self {
        self.replace(Self::new())
    }

    #[inline(always)]
    pub fn set_option_cstr(&self, s: &std::ffi::CStr) {
        self.0.set(Some(intern_opt_cstr(s)));
    }

    #[inline(always)]
    pub fn set_option_bytes(&self, bytes: &[u8]) {
        let bytes = crate::memory::bytes_until_nul(bytes);
        self.set_option_cstr(&crate::memory::bytes_to_cstring(bytes));
    }

    #[inline(always)]
    pub fn clear_option_str(&self) {
        self.0.set(None);
    }

    #[inline(always)]
    pub const fn check_option_str(&self) {}

    #[inline(always)]
    pub fn as_cstr(&self) -> &'static std::ffi::CStr {
        self.0.get().unwrap_or(c"")
    }

    #[inline(always)]
    pub fn as_bytes(&self) -> &'static [u8] {
        self.as_cstr().to_bytes()
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.as_bytes().is_empty()
    }

    #[inline(always)]
    pub fn is_null(&self) -> bool {
        self.0.get().is_none()
    }

    #[inline(always)]
    pub fn first_byte(&self) -> u8 {
        self.as_bytes().first().copied().unwrap_or(0)
    }

    #[inline(always)]
    pub fn as_ptr(&self) -> *const std::ffi::c_char {
        self.as_cstr().as_ptr()
    }

    #[inline(always)]
    pub fn to_owned(&self) -> std::ffi::CString {
        self.as_cstr().to_owned()
    }
}

impl std::ops::Deref for OptString {
    type Target = std::ffi::CStr;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.as_cstr()
    }
}

#[repr(C)]
pub struct Synblock {
    pub b_keywtab: std::collections::BTreeMap<Vec<u8>, Vec<Keyentry>>,
    pub b_keywtab_ic: std::collections::BTreeMap<Vec<u8>, Vec<Keyentry>>,
    pub b_syn_error: bool,
    pub b_syn_slow: bool,
    pub b_syn_ic: bool,
    pub b_syn_foldlevel: i32,
    pub b_syn_spell: i32,
    pub b_syn_patterns: Vec<Synpat>,
    pub b_syn_clusters: Vec<SynCluster>,
    pub b_spell_cluster_id: i32,
    pub b_nospell_cluster_id: i32,
    pub b_syn_containedin: i32,
    pub b_syn_sync_flags: i32,
    pub b_syn_sync_id: i16,
    pub b_syn_sync_minlines: i32,
    pub b_syn_sync_maxlines: i32,
    pub b_syn_sync_linebreaks: i32,
    pub b_syn_linecont_pat: Option<std::ffi::CString>,
    pub b_syn_linecont_prog: Option<Box<Regprog>>,
    pub b_syn_linecont_time: SynTime,
    pub b_syn_linecont_ic: bool,
    pub b_syn_topgrp: i32,
    pub b_syn_conceal: i32,
    pub b_syn_folditems: i32,
    pub b_sst_array: Vec<Synstate>,
    pub b_sst_max: i32,
    pub b_sst_search: usize,
    pub b_sst_check_lnum: i32,
    pub b_sst_lasttick: u64,
    pub b_idlist_cache: Option<Box<IdlCache>>,
    pub b_langp: Vec<crate::spellsuggest::Langp>,
    pub b_spell_ismw: [bool; 256],
    pub b_spell_ismw_mb: Option<std::ffi::CString>,
    pub b_p_spc: OptString,
    pub b_cap_prog: Option<Box<Regprog>>,
    pub b_p_spf: OptString,
    pub b_p_spl: OptString,
    pub b_p_spo: OptString,
    pub b_p_spo_flags: u32,
    pub b_cjk: i32,
    pub b_syn_chartab: [u64; 4],
    pub b_syn_isk: Option<std::ffi::CString>,
}

impl Synblock {
    pub const fn new() -> Self {
        Self {
            b_keywtab: std::collections::BTreeMap::new(),
            b_keywtab_ic: std::collections::BTreeMap::new(),
            b_syn_error: false,
            b_syn_slow: false,
            b_syn_ic: false,
            b_syn_foldlevel: 0,
            b_syn_spell: 0,
            b_syn_patterns: Vec::new(),
            b_syn_clusters: Vec::new(),
            b_spell_cluster_id: 0,
            b_nospell_cluster_id: 0,
            b_syn_containedin: 0,
            b_syn_sync_flags: 0,
            b_syn_sync_id: 0,
            b_syn_sync_minlines: 0,
            b_syn_sync_maxlines: 0,
            b_syn_sync_linebreaks: 0,
            b_syn_linecont_pat: None,
            b_syn_linecont_prog: None,
            b_syn_linecont_time: SynTime {
                total: 0,
                slowest: 0,
                count: 0,
                match_: 0,
            },
            b_syn_linecont_ic: false,
            b_syn_topgrp: 0,
            b_syn_conceal: 0,
            b_syn_folditems: 0,
            b_sst_array: Vec::new(),
            b_sst_max: 0,
            b_sst_search: 0,
            b_sst_check_lnum: 0,
            b_sst_lasttick: 0,
            b_idlist_cache: None,
            b_langp: Vec::new(),
            b_spell_ismw: [false; 256],
            b_spell_ismw_mb: None,
            b_p_spc: OptString::new(),
            b_cap_prog: None,
            b_p_spf: OptString::new(),
            b_p_spl: OptString::new(),
            b_p_spo: OptString::new(),
            b_p_spo_flags: 0,
            b_cjk: 0,
            b_syn_chartab: [0; 4],
            b_syn_isk: None,
        }
    }
}

#[repr(C)]
pub struct Winopt {
    pub wo_arab: bool,
    pub wo_bri: bool,
    pub wo_briopt: OptString,
    pub wo_diff: bool,
    pub wo_fdc: OptString,
    pub wo_eiw: OptString,
    pub wo_fdc_save: OptString,
    pub wo_fen: bool,
    pub wo_fen_save: bool,
    pub wo_fdi: OptString,
    pub wo_fdl: i64,
    pub wo_fdl_save: i64,
    pub wo_fdm: OptString,
    pub wo_fdm_save: OptString,
    pub wo_fml: i64,
    pub wo_fdn: i64,
    pub wo_fde: crate::eval::typval::Callback,
    pub wo_fdt: crate::eval::typval::Callback,
    pub wo_fmr: OptString,
    pub wo_lbr: bool,
    pub wo_list: bool,
    pub wo_nu: bool,
    pub wo_rnu: bool,
    pub wo_ve: OptString,
    pub wo_ve_flags: u32,
    pub wo_nuw: i64,
    pub wo_wfb: bool,
    pub wo_wfh: bool,
    pub wo_wfw: bool,
    pub wo_wp: bool,
    pub wo_pvw: bool,
    pub wo_lhi: i64,
    pub wo_rl: bool,
    pub wo_rlc: OptString,
    pub wo_scr: i64,
    pub wo_sms: bool,
    pub wo_spell: bool,
    pub wo_cuc: bool,
    pub wo_cul: bool,
    pub wo_culopt: OptString,
    pub wo_cc: OptString,
    pub wo_sbr: OptString,
    pub wo_stc: OptString,
    pub wo_stl: OptString,
    pub wo_wbr: OptString,
    pub wo_scb: bool,
    pub wo_diff_saved: bool,
    pub wo_scb_save: bool,
    pub wo_wrap: bool,
    pub wo_wrap_save: bool,
    pub wo_cocu: OptString,
    pub wo_cole: i64,
    pub wo_crb: bool,
    pub wo_crb_save: bool,
    pub wo_scl: OptString,
    pub wo_siso: i64,
    pub wo_so: i64,
    pub wo_sop: i64,
    pub wo_winhl: OptString,
    pub wo_lcs: OptString,
    pub wo_fcs: OptString,
    pub wo_winbl: i64,
    pub wo_wrap_flags: u32,
    pub wo_stl_flags: u32,
    pub wo_wbr_flags: u32,
    pub wo_fde_flags: u32,
    pub wo_fdt_flags: u32,
    pub wo_script_ctx: [crate::runtime::Sctx; K_WIN_OPT_COUNT],
}

impl Winopt {
    #[allow(clippy::declare_interior_mutable_const)]
    pub const ZERO: Self = Self {
        wo_arab: false,
        wo_bri: false,
        wo_briopt: OptString::new(),
        wo_diff: false,
        wo_fdc: OptString::new(),
        wo_eiw: OptString::new(),
        wo_fdc_save: OptString::new(),
        wo_fen: false,
        wo_fen_save: false,
        wo_fdi: OptString::new(),
        wo_fdl: 0,
        wo_fdl_save: 0,
        wo_fdm: OptString::new(),
        wo_fdm_save: OptString::new(),
        wo_fml: 0,
        wo_fdn: 0,
        wo_fde: crate::eval::typval::Callback::NONE,
        wo_fdt: crate::eval::typval::Callback::NONE,
        wo_fmr: OptString::new(),
        wo_lbr: false,
        wo_list: false,
        wo_nu: false,
        wo_rnu: false,
        wo_ve: OptString::new(),
        wo_ve_flags: 0,
        wo_nuw: 0,
        wo_wfb: false,
        wo_wfh: false,
        wo_wfw: false,
        wo_wp: false,
        wo_pvw: false,
        wo_lhi: 0,
        wo_rl: false,
        wo_rlc: OptString::new(),
        wo_scr: 0,
        wo_sms: false,
        wo_spell: false,
        wo_cuc: false,
        wo_cul: false,
        wo_culopt: OptString::new(),
        wo_cc: OptString::new(),
        wo_sbr: OptString::new(),
        wo_stc: OptString::new(),
        wo_stl: OptString::new(),
        wo_wbr: OptString::new(),
        wo_scb: false,
        wo_diff_saved: false,
        wo_scb_save: false,
        wo_wrap: false,
        wo_wrap_save: false,
        wo_cocu: OptString::new(),
        wo_cole: 0,
        wo_crb: false,
        wo_crb_save: false,
        wo_scl: OptString::new(),
        wo_siso: 0,
        wo_so: 0,
        wo_sop: 0,
        wo_winhl: OptString::new(),
        wo_lcs: OptString::new(),
        wo_fcs: OptString::new(),
        wo_winbl: 0,
        wo_wrap_flags: 0,
        wo_stl_flags: 0,
        wo_wbr_flags: 0,
        wo_fde_flags: 0,
        wo_fdt_flags: 0,
        wo_script_ctx: [crate::runtime::Sctx {
            sc_sid: 0,
            sc_seq: 0,
            sc_lnum: 0,
            sc_chan: 0,
        }; K_WIN_OPT_COUNT],
    };
}

impl Drop for Winopt {
    fn drop(&mut self) {
        crate::option::clear_winopt(self);
    }
}

pub struct OptionVars {
    pub p_ambw: OptString,
    pub p_acd: std::cell::Cell<bool>,
    pub p_ai: std::cell::Cell<bool>,
    pub p_bin: std::cell::Cell<bool>,
    pub p_bomb: std::cell::Cell<bool>,
    pub p_bl: std::cell::Cell<bool>,
    pub p_cin: std::cell::Cell<bool>,
    pub p_channel: std::cell::Cell<OptInt>,
    pub p_cink: OptString,
    pub p_cinsd: OptString,
    pub p_cinw: OptString,
    pub p_cfu: Callback,
    pub p_ofu: Callback,
    pub p_tsrfu: Callback,
    pub p_ci: std::cell::Cell<bool>,
    pub p_ar: std::cell::Cell<bool>,
    pub p_aw: std::cell::Cell<bool>,
    pub p_awa: std::cell::Cell<bool>,
    pub p_bs: OptString,
    pub p_bg: OptString,
    pub p_bk: std::cell::Cell<bool>,
    pub p_bkc: OptString,
    pub bkc_flags: std::cell::Cell<u32>,
    pub p_bdir: OptString,
    pub p_bex: OptString,
    pub p_bo: OptString,
    pub breakat_flags: [std::cell::Cell<bool>; 256],
    pub bo_flags: std::cell::Cell<u32>,
    pub p_bsk: OptString,
    pub p_breakat: OptString,
    pub p_bh: OptString,
    pub p_bt: OptString,
    pub p_busy: std::cell::Cell<OptInt>,
    pub p_cmp: OptString,
    pub cmp_flags: std::cell::Cell<u32>,
    pub p_enc: OptString,
    pub p_deco: std::cell::Cell<bool>,
    pub p_ccv: Callback,
    pub p_cino: OptString,
    pub p_cedit: OptString,
    pub p_cb: OptString,
    pub cb_flags: std::cell::Cell<u32>,
    pub p_cwh: std::cell::Cell<OptInt>,
    pub p_ch: std::cell::Cell<OptInt>,
    pub p_cms: OptString,
    pub p_cpt: OptString,
    pub p_cto: std::cell::Cell<OptInt>,
    pub p_columns: std::cell::Cell<OptInt>,
    pub p_confirm: std::cell::Cell<bool>,
    pub p_cia: OptString,
    pub cia_flags: std::cell::Cell<u32>,
    pub p_cot: OptString,
    pub cot_flags: std::cell::Cell<u32>,
    pub p_ac: std::cell::Cell<bool>,
    pub p_act: std::cell::Cell<OptInt>,
    pub p_acl: std::cell::Cell<OptInt>,
    #[cfg(windows)]
    pub p_csl: OptString,
    pub p_pumborder: OptString,
    pub p_pb: std::cell::Cell<OptInt>,
    pub p_ph: std::cell::Cell<OptInt>,
    pub p_pw: std::cell::Cell<OptInt>,
    pub p_pmw: std::cell::Cell<OptInt>,
    pub p_com: OptString,
    pub p_cpo: OptString,
    pub p_debug: OptString,
    pub p_def: OptString,
    pub p_inc: OptString,
    pub p_dia: OptString,
    pub p_dip: OptString,
    pub p_dex: Callback,
    pub p_dict: OptString,
    pub p_dg: std::cell::Cell<bool>,
    pub p_dir: OptString,
    pub p_dy: OptString,
    pub dy_flags: std::cell::Cell<u32>,
    pub dy_escape_width: std::cell::Cell<i32>,
    pub p_ead: OptString,
    pub p_emoji: std::cell::Cell<bool>,
    pub p_ea: std::cell::Cell<bool>,
    pub p_ep: OptString,
    pub p_eb: std::cell::Cell<bool>,
    pub p_ef: OptString,
    pub p_efm: OptString,
    pub p_gefm: OptString,
    pub p_gp: OptString,
    pub p_eof: std::cell::Cell<bool>,
    pub p_eol: std::cell::Cell<bool>,
    pub p_ei: OptString,
    pub p_et: std::cell::Cell<bool>,
    pub p_exrc: std::cell::Cell<bool>,
    pub p_fenc: OptString,
    pub p_fencs: OptString,
    pub p_ff: OptString,
    pub p_ffs: OptString,
    pub p_fic: std::cell::Cell<bool>,
    pub p_ft: OptString,
    pub p_fcs: OptString,
    pub p_ffu: Callback,
    pub p_fixeol: std::cell::Cell<bool>,
    pub p_fcl: OptString,
    pub p_fdls: std::cell::Cell<OptInt>,
    pub p_fdo: OptString,
    pub fdo_flags: std::cell::Cell<u32>,
    pub p_follow: std::cell::Cell<bool>,
    pub p_fex: Callback,
    pub p_flp: OptString,
    pub p_fo: OptString,
    pub p_fp: OptString,
    pub p_fs: std::cell::Cell<bool>,
    pub p_gd: std::cell::Cell<bool>,
    pub p_guicursor: OptString,
    pub p_guifont: OptString,
    pub p_guifontwide: OptString,
    pub p_hf: OptString,
    pub p_hh: std::cell::Cell<OptInt>,
    pub p_hlg: OptString,
    pub p_hid: std::cell::Cell<bool>,
    pub p_hl: OptString,
    pub p_hls: std::cell::Cell<bool>,
    pub p_hi: std::cell::Cell<OptInt>,
    pub p_arshape: std::cell::Cell<bool>,
    pub p_icon: std::cell::Cell<bool>,
    pub p_iconstring: OptString,
    pub p_ic: std::cell::Cell<bool>,
    pub p_iminsert: std::cell::Cell<OptInt>,
    pub p_imsearch: std::cell::Cell<OptInt>,
    pub p_inf: std::cell::Cell<bool>,
    pub p_inex: Callback,
    pub p_is: std::cell::Cell<bool>,
    pub p_inde: Callback,
    pub p_indk: OptString,
    pub p_icm: OptString,
    pub p_isf: OptString,
    pub p_isi: OptString,
    pub p_isk: OptString,
    pub p_js: std::cell::Cell<bool>,
    pub p_jop: OptString,
    pub jop_flags: std::cell::Cell<u32>,
    pub p_keymap: OptString,
    pub p_kp: OptString,
    pub p_km: OptString,
    pub p_langmap: OptString,
    pub p_lnr: std::cell::Cell<bool>,
    pub p_lrm: std::cell::Cell<bool>,
    pub p_lm: OptString,
    pub p_lines: std::cell::Cell<OptInt>,
    pub p_linespace: std::cell::Cell<OptInt>,
    pub p_lisp: std::cell::Cell<bool>,
    pub p_lop: OptString,
    pub p_lispwords: OptString,
    pub p_ls: std::cell::Cell<OptInt>,
    pub p_stal: std::cell::Cell<OptInt>,
    pub p_lcs: OptString,
    pub p_lz: std::cell::Cell<bool>,
    pub p_lpl: std::cell::Cell<bool>,
    pub p_magic: std::cell::Cell<bool>,
    pub p_menc: OptString,
    pub p_mef: OptString,
    pub p_mp: OptString,
    pub p_mps: OptString,
    pub p_mat: std::cell::Cell<OptInt>,
    pub p_mco: std::cell::Cell<OptInt>,
    pub p_mfd: std::cell::Cell<OptInt>,
    pub p_mmd: std::cell::Cell<OptInt>,
    pub p_mmp: std::cell::Cell<OptInt>,
    pub p_mis: std::cell::Cell<OptInt>,
    pub p_mopt: OptString,
    pub p_msc: std::cell::Cell<OptInt>,
    pub p_msm: OptString,
    pub p_ml: std::cell::Cell<bool>,
    pub p_mle: std::cell::Cell<bool>,
    pub p_mls: std::cell::Cell<OptInt>,
    pub p_ma: std::cell::Cell<bool>,
    pub p_mod: std::cell::Cell<bool>,
    pub p_mouse: OptString,
    pub p_mousem: OptString,
    pub p_mousemev: std::cell::Cell<bool>,
    pub p_mousef: std::cell::Cell<bool>,
    pub p_mh: std::cell::Cell<bool>,
    pub p_mousescroll: OptString,
    pub p_mousescroll_vert: std::cell::Cell<OptInt>,
    pub p_mousescroll_hor: std::cell::Cell<OptInt>,
    pub p_mouset: std::cell::Cell<OptInt>,
    pub p_more: std::cell::Cell<bool>,
    pub p_nf: OptString,
    pub p_opfunc: Callback,
    pub p_para: OptString,
    pub p_paste: std::cell::Cell<bool>,
    pub p_pex: Callback,
    pub p_pm: OptString,
    pub p_path: OptString,
    pub p_cdpath: OptString,
    pub p_pi: std::cell::Cell<bool>,
    pub p_pyx: std::cell::Cell<OptInt>,
    pub p_qe: OptString,
    pub p_ro: std::cell::Cell<bool>,
    pub p_rdb: OptString,
    pub rdb_flags: std::cell::Cell<u32>,
    pub p_rdt: std::cell::Cell<OptInt>,
    pub p_re: std::cell::Cell<OptInt>,
    pub p_report: std::cell::Cell<OptInt>,
    pub p_pvp: OptString,
    pub p_pvh: std::cell::Cell<OptInt>,
    pub p_chi: std::cell::Cell<OptInt>,
    pub p_ari: std::cell::Cell<bool>,
    pub p_ri: std::cell::Cell<bool>,
    pub p_ru: std::cell::Cell<bool>,
    pub p_ruf: OptString,
    pub p_plf: OptString,
    pub p_pp: OptString,
    pub p_qftf: Callback,
    pub p_rtp: OptString,
    pub p_scbk: std::cell::Cell<OptInt>,
    pub p_sj: std::cell::Cell<OptInt>,
    pub p_so: std::cell::Cell<OptInt>,
    pub p_sop: std::cell::Cell<OptInt>,
    pub p_sbo: OptString,
    pub p_sections: OptString,
    pub p_secure: std::cell::Cell<bool>,
    pub p_sel: OptString,
    pub p_slm: OptString,
    pub p_ssop: OptString,
    pub ssop_flags: std::cell::Cell<u32>,
    pub p_sh: OptString,
    pub p_shcf: OptString,
    pub p_sp: OptString,
    pub p_shq: OptString,
    pub p_sxq: OptString,
    pub p_sxe: OptString,
    pub p_srr: OptString,
    pub p_stmp: std::cell::Cell<bool>,
    #[cfg(windows)]
    pub p_ssl: std::cell::Cell<bool>,
    pub p_stl: OptString,
    pub p_wbr: OptString,
    pub p_sr: std::cell::Cell<bool>,
    pub p_sw: std::cell::Cell<OptInt>,
    pub p_shm: OptString,
    pub p_sbr: OptString,
    pub p_sc: std::cell::Cell<bool>,
    pub p_sloc: OptString,
    pub p_sft: std::cell::Cell<bool>,
    pub p_sm: std::cell::Cell<bool>,
    pub p_smd: std::cell::Cell<bool>,
    pub p_ss: std::cell::Cell<OptInt>,
    pub p_siso: std::cell::Cell<OptInt>,
    pub p_scs: std::cell::Cell<bool>,
    pub p_si: std::cell::Cell<bool>,
    pub p_sta: std::cell::Cell<bool>,
    pub p_sts: std::cell::Cell<OptInt>,
    pub p_sb: std::cell::Cell<bool>,
    pub p_sua: OptString,
    pub p_swf: std::cell::Cell<bool>,
    pub p_smc: std::cell::Cell<OptInt>,
    pub p_tpm: std::cell::Cell<OptInt>,
    pub p_tal: OptString,
    pub p_tpf: OptString,
    pub tpf_flags: std::cell::Cell<u32>,
    pub p_tfu: Callback,
    pub p_spc: OptString,
    pub p_spf: OptString,
    pub p_spl: OptString,
    pub p_spo: OptString,
    pub spo_flags: std::cell::Cell<u32>,
    pub p_sps: OptString,
    pub p_spr: std::cell::Cell<bool>,
    pub p_sol: std::cell::Cell<bool>,
    pub p_su: OptString,
    pub p_swb: OptString,
    pub swb_flags: std::cell::Cell<u32>,
    pub p_spk: OptString,
    pub p_syn: OptString,
    pub p_tcl: OptString,
    pub tcl_flags: std::cell::Cell<u32>,
    pub p_ts: std::cell::Cell<OptInt>,
    pub p_tbs: std::cell::Cell<bool>,
    pub p_tc: OptString,
    pub tc_flags: std::cell::Cell<u32>,
    pub p_tl: std::cell::Cell<OptInt>,
    pub p_tr: std::cell::Cell<bool>,
    pub p_tags: OptString,
    pub p_tgst: std::cell::Cell<bool>,
    pub p_tbidi: std::cell::Cell<bool>,
    pub p_tw: std::cell::Cell<OptInt>,
    pub p_to: std::cell::Cell<bool>,
    pub p_timeout: std::cell::Cell<bool>,
    pub p_tm: std::cell::Cell<OptInt>,
    pub p_title: std::cell::Cell<bool>,
    pub p_titlelen: std::cell::Cell<OptInt>,
    pub p_titleold: OptString,
    pub p_titlestring: OptString,
    pub p_tsr: OptString,
    pub p_tgc: std::cell::Cell<bool>,
    pub p_ttimeout: std::cell::Cell<bool>,
    pub p_ttm: std::cell::Cell<OptInt>,
    pub p_tf: std::cell::Cell<bool>,
    pub p_udir: OptString,
    pub p_udf: std::cell::Cell<bool>,
    pub p_ul: std::cell::Cell<OptInt>,
    pub p_ur: std::cell::Cell<OptInt>,
    pub p_uc: std::cell::Cell<OptInt>,
    pub p_ut: std::cell::Cell<OptInt>,
    pub p_shada: OptString,
    pub p_shadafile: OptString,
    pub p_termsync: std::cell::Cell<bool>,
    pub p_vsts: OptString,
    pub p_vts: OptString,
    pub p_vdir: OptString,
    pub p_vop: OptString,
    pub vop_flags: std::cell::Cell<u32>,
    pub p_vb: std::cell::Cell<bool>,
    pub p_ve: OptString,
    pub ve_flags: std::cell::Cell<u32>,
    pub p_verbose: std::cell::Cell<OptInt>,
    pub p_vfile: OptString,
    pub p_warn: std::cell::Cell<bool>,
    pub p_wop: OptString,
    pub wop_flags: std::cell::Cell<u32>,
    pub p_window: std::cell::Cell<OptInt>,
    pub p_wak: OptString,
    pub p_wig: OptString,
    pub p_ww: OptString,
    pub p_wc: std::cell::Cell<OptInt>,
    pub p_wcm: std::cell::Cell<OptInt>,
    pub p_wic: std::cell::Cell<bool>,
    pub p_wim: OptString,
    pub p_wmnu: std::cell::Cell<bool>,
    pub p_winborder: OptString,
    pub p_wh: std::cell::Cell<OptInt>,
    pub p_wmh: std::cell::Cell<OptInt>,
    pub p_wmw: std::cell::Cell<OptInt>,
    pub p_wiw: std::cell::Cell<OptInt>,
    pub p_wm: std::cell::Cell<OptInt>,
    pub p_ws: std::cell::Cell<bool>,
    pub p_write: std::cell::Cell<bool>,
    pub p_wa: std::cell::Cell<bool>,
    pub p_wb: std::cell::Cell<bool>,
    pub p_wd: std::cell::Cell<OptInt>,
    pub p_cdh: std::cell::Cell<bool>,
}

impl OptionVars {
    pub const fn new() -> Self {
        Self {
            p_ambw: OptString::new(),
            p_acd: std::cell::Cell::new(false),
            p_ai: std::cell::Cell::new(false),
            p_bin: std::cell::Cell::new(false),
            p_bomb: std::cell::Cell::new(false),
            p_bl: std::cell::Cell::new(false),
            p_cin: std::cell::Cell::new(false),
            p_channel: std::cell::Cell::new(0),
            p_cink: OptString::new(),
            p_cinsd: OptString::new(),
            p_cinw: OptString::new(),
            p_cfu: Callback::NONE,
            p_ofu: Callback::NONE,
            p_tsrfu: Callback::NONE,
            p_ci: std::cell::Cell::new(false),
            p_ar: std::cell::Cell::new(false),
            p_aw: std::cell::Cell::new(false),
            p_awa: std::cell::Cell::new(false),
            p_bs: OptString::new(),
            p_bg: OptString::new(),
            p_bk: std::cell::Cell::new(false),
            p_bkc: OptString::new(),
            bkc_flags: std::cell::Cell::new(0),
            p_bdir: OptString::new(),
            p_bex: OptString::new(),
            p_bo: OptString::new(),
            breakat_flags: [const { std::cell::Cell::new(false) }; 256],
            bo_flags: std::cell::Cell::new(0),
            p_bsk: OptString::new(),
            p_breakat: OptString::new(),
            p_bh: OptString::new(),
            p_bt: OptString::new(),
            p_busy: std::cell::Cell::new(0),
            p_cmp: OptString::new(),
            cmp_flags: std::cell::Cell::new(0),
            p_enc: OptString::new(),
            p_deco: std::cell::Cell::new(false),
            p_ccv: Callback::NONE,
            p_cino: OptString::new(),
            p_cedit: OptString::new(),
            p_cb: OptString::new(),
            cb_flags: std::cell::Cell::new(0),
            p_cwh: std::cell::Cell::new(0),
            p_ch: std::cell::Cell::new(0),
            p_cms: OptString::new(),
            p_cpt: OptString::new(),
            p_cto: std::cell::Cell::new(0),
            p_columns: std::cell::Cell::new(0),
            p_confirm: std::cell::Cell::new(false),
            p_cia: OptString::new(),
            cia_flags: std::cell::Cell::new(0),
            p_cot: OptString::new(),
            cot_flags: std::cell::Cell::new(0),
            p_ac: std::cell::Cell::new(false),
            p_act: std::cell::Cell::new(0),
            p_acl: std::cell::Cell::new(0),
            #[cfg(windows)]
            p_csl: OptString::new(),
            p_pumborder: OptString::new(),
            p_pb: std::cell::Cell::new(0),
            p_ph: std::cell::Cell::new(0),
            p_pw: std::cell::Cell::new(0),
            p_pmw: std::cell::Cell::new(0),
            p_com: OptString::new(),
            p_cpo: OptString::new(),
            p_debug: OptString::new(),
            p_def: OptString::new(),
            p_inc: OptString::new(),
            p_dia: OptString::new(),
            p_dip: OptString::new(),
            p_dex: Callback::NONE,
            p_dict: OptString::new(),
            p_dg: std::cell::Cell::new(false),
            p_dir: OptString::new(),
            p_dy: OptString::new(),
            dy_flags: std::cell::Cell::new(0),
            dy_escape_width: std::cell::Cell::new(2),
            p_ead: OptString::new(),
            p_emoji: std::cell::Cell::new(false),
            p_ea: std::cell::Cell::new(false),
            p_ep: OptString::new(),
            p_eb: std::cell::Cell::new(false),
            p_ef: OptString::new(),
            p_efm: OptString::new(),
            p_gefm: OptString::new(),
            p_gp: OptString::new(),
            p_eof: std::cell::Cell::new(false),
            p_eol: std::cell::Cell::new(false),
            p_ei: OptString::new(),
            p_et: std::cell::Cell::new(false),
            p_exrc: std::cell::Cell::new(false),
            p_fenc: OptString::new(),
            p_fencs: OptString::new(),
            p_ff: OptString::new(),
            p_ffs: OptString::new(),
            p_fic: std::cell::Cell::new(false),
            p_ft: OptString::new(),
            p_fcs: OptString::new(),
            p_ffu: Callback::NONE,
            p_fixeol: std::cell::Cell::new(false),
            p_fcl: OptString::new(),
            p_fdls: std::cell::Cell::new(0),
            p_fdo: OptString::new(),
            fdo_flags: std::cell::Cell::new(0),
            p_follow: std::cell::Cell::new(false),
            p_fex: Callback::NONE,
            p_flp: OptString::new(),
            p_fo: OptString::new(),
            p_fp: OptString::new(),
            p_fs: std::cell::Cell::new(false),
            p_gd: std::cell::Cell::new(false),
            p_guicursor: OptString::new(),
            p_guifont: OptString::new(),
            p_guifontwide: OptString::new(),
            p_hf: OptString::new(),
            p_hh: std::cell::Cell::new(0),
            p_hlg: OptString::new(),
            p_hid: std::cell::Cell::new(false),
            p_hl: OptString::new(),
            p_hls: std::cell::Cell::new(false),
            p_hi: std::cell::Cell::new(0),
            p_arshape: std::cell::Cell::new(false),
            p_icon: std::cell::Cell::new(false),
            p_iconstring: OptString::new(),
            p_ic: std::cell::Cell::new(false),
            p_iminsert: std::cell::Cell::new(0),
            p_imsearch: std::cell::Cell::new(0),
            p_inf: std::cell::Cell::new(false),
            p_inex: Callback::NONE,
            p_is: std::cell::Cell::new(false),
            p_inde: Callback::NONE,
            p_indk: OptString::new(),
            p_icm: OptString::new(),
            p_isf: OptString::new(),
            p_isi: OptString::new(),
            p_isk: OptString::new(),
            p_js: std::cell::Cell::new(false),
            p_jop: OptString::new(),
            jop_flags: std::cell::Cell::new(0),
            p_keymap: OptString::new(),
            p_kp: OptString::new(),
            p_km: OptString::new(),
            p_langmap: OptString::new(),
            p_lnr: std::cell::Cell::new(false),
            p_lrm: std::cell::Cell::new(false),
            p_lm: OptString::new(),
            p_lines: std::cell::Cell::new(0),
            p_linespace: std::cell::Cell::new(0),
            p_lisp: std::cell::Cell::new(false),
            p_lop: OptString::new(),
            p_lispwords: OptString::new(),
            p_ls: std::cell::Cell::new(0),
            p_stal: std::cell::Cell::new(0),
            p_lcs: OptString::new(),
            p_lz: std::cell::Cell::new(false),
            p_lpl: std::cell::Cell::new(false),
            p_magic: std::cell::Cell::new(false),
            p_menc: OptString::new(),
            p_mef: OptString::new(),
            p_mp: OptString::new(),
            p_mps: OptString::new(),
            p_mat: std::cell::Cell::new(0),
            p_mco: std::cell::Cell::new(0),
            p_mfd: std::cell::Cell::new(0),
            p_mmd: std::cell::Cell::new(0),
            p_mmp: std::cell::Cell::new(0),
            p_mis: std::cell::Cell::new(0),
            p_mopt: OptString::new(),
            p_msc: std::cell::Cell::new(0),
            p_msm: OptString::new(),
            p_ml: std::cell::Cell::new(false),
            p_mle: std::cell::Cell::new(false),
            p_mls: std::cell::Cell::new(0),
            p_ma: std::cell::Cell::new(false),
            p_mod: std::cell::Cell::new(false),
            p_mouse: OptString::new(),
            p_mousem: OptString::new(),
            p_mousemev: std::cell::Cell::new(false),
            p_mousef: std::cell::Cell::new(false),
            p_mh: std::cell::Cell::new(false),
            p_mousescroll: OptString::new(),
            p_mousescroll_vert: std::cell::Cell::new(3),
            p_mousescroll_hor: std::cell::Cell::new(6),
            p_mouset: std::cell::Cell::new(0),
            p_more: std::cell::Cell::new(false),
            p_nf: OptString::new(),
            p_opfunc: Callback::NONE,
            p_para: OptString::new(),
            p_paste: std::cell::Cell::new(false),
            p_pex: Callback::NONE,
            p_pm: OptString::new(),
            p_path: OptString::new(),
            p_cdpath: OptString::new(),
            p_pi: std::cell::Cell::new(false),
            p_pyx: std::cell::Cell::new(0),
            p_qe: OptString::new(),
            p_ro: std::cell::Cell::new(false),
            p_rdb: OptString::new(),
            rdb_flags: std::cell::Cell::new(0),
            p_rdt: std::cell::Cell::new(0),
            p_re: std::cell::Cell::new(0),
            p_report: std::cell::Cell::new(0),
            p_pvp: OptString::new(),
            p_pvh: std::cell::Cell::new(0),
            p_chi: std::cell::Cell::new(0),
            p_ari: std::cell::Cell::new(false),
            p_ri: std::cell::Cell::new(false),
            p_ru: std::cell::Cell::new(false),
            p_ruf: OptString::new(),
            p_plf: OptString::new(),
            p_pp: OptString::new(),
            p_qftf: Callback::NONE,
            p_rtp: OptString::new(),
            p_scbk: std::cell::Cell::new(0),
            p_sj: std::cell::Cell::new(0),
            p_so: std::cell::Cell::new(0),
            p_sop: std::cell::Cell::new(0),
            p_sbo: OptString::new(),
            p_sections: OptString::new(),
            p_secure: std::cell::Cell::new(false),
            p_sel: OptString::new(),
            p_slm: OptString::new(),
            p_ssop: OptString::new(),
            ssop_flags: std::cell::Cell::new(0),
            p_sh: OptString::new(),
            p_shcf: OptString::new(),
            p_sp: OptString::new(),
            p_shq: OptString::new(),
            p_sxq: OptString::new(),
            p_sxe: OptString::new(),
            p_srr: OptString::new(),
            p_stmp: std::cell::Cell::new(false),
            #[cfg(windows)]
            p_ssl: std::cell::Cell::new(false),
            p_stl: OptString::new(),
            p_wbr: OptString::new(),
            p_sr: std::cell::Cell::new(false),
            p_sw: std::cell::Cell::new(0),
            p_shm: OptString::new(),
            p_sbr: OptString::new(),
            p_sc: std::cell::Cell::new(false),
            p_sloc: OptString::new(),
            p_sft: std::cell::Cell::new(false),
            p_sm: std::cell::Cell::new(false),
            p_smd: std::cell::Cell::new(false),
            p_ss: std::cell::Cell::new(0),
            p_siso: std::cell::Cell::new(0),
            p_scs: std::cell::Cell::new(false),
            p_si: std::cell::Cell::new(false),
            p_sta: std::cell::Cell::new(false),
            p_sts: std::cell::Cell::new(0),
            p_sb: std::cell::Cell::new(false),
            p_sua: OptString::new(),
            p_swf: std::cell::Cell::new(false),
            p_smc: std::cell::Cell::new(0),
            p_tpm: std::cell::Cell::new(0),
            p_tal: OptString::new(),
            p_tpf: OptString::new(),
            tpf_flags: std::cell::Cell::new(0),
            p_tfu: Callback::NONE,
            p_spc: OptString::new(),
            p_spf: OptString::new(),
            p_spl: OptString::new(),
            p_spo: OptString::new(),
            spo_flags: std::cell::Cell::new(0),
            p_sps: OptString::new(),
            p_spr: std::cell::Cell::new(false),
            p_sol: std::cell::Cell::new(false),
            p_su: OptString::new(),
            p_swb: OptString::new(),
            swb_flags: std::cell::Cell::new(0),
            p_spk: OptString::new(),
            p_syn: OptString::new(),
            p_tcl: OptString::new(),
            tcl_flags: std::cell::Cell::new(0),
            p_ts: std::cell::Cell::new(0),
            p_tbs: std::cell::Cell::new(false),
            p_tc: OptString::new(),
            tc_flags: std::cell::Cell::new(0),
            p_tl: std::cell::Cell::new(0),
            p_tr: std::cell::Cell::new(false),
            p_tags: OptString::new(),
            p_tgst: std::cell::Cell::new(false),
            p_tbidi: std::cell::Cell::new(false),
            p_tw: std::cell::Cell::new(0),
            p_to: std::cell::Cell::new(false),
            p_timeout: std::cell::Cell::new(false),
            p_tm: std::cell::Cell::new(0),
            p_title: std::cell::Cell::new(false),
            p_titlelen: std::cell::Cell::new(0),
            p_titleold: OptString::new(),
            p_titlestring: OptString::new(),
            p_tsr: OptString::new(),
            p_tgc: std::cell::Cell::new(false),
            p_ttimeout: std::cell::Cell::new(false),
            p_ttm: std::cell::Cell::new(0),
            p_tf: std::cell::Cell::new(false),
            p_udir: OptString::new(),
            p_udf: std::cell::Cell::new(false),
            p_ul: std::cell::Cell::new(0),
            p_ur: std::cell::Cell::new(0),
            p_uc: std::cell::Cell::new(0),
            p_ut: std::cell::Cell::new(0),
            p_shada: OptString::new(),
            p_shadafile: OptString::new(),
            p_termsync: std::cell::Cell::new(false),
            p_vsts: OptString::new(),
            p_vts: OptString::new(),
            p_vdir: OptString::new(),
            p_vop: OptString::new(),
            vop_flags: std::cell::Cell::new(0),
            p_vb: std::cell::Cell::new(false),
            p_ve: OptString::new(),
            ve_flags: std::cell::Cell::new(0),
            p_verbose: std::cell::Cell::new(0),
            p_vfile: OptString::new(),
            p_warn: std::cell::Cell::new(false),
            p_wop: OptString::new(),
            wop_flags: std::cell::Cell::new(0),
            p_window: std::cell::Cell::new(0),
            p_wak: OptString::new(),
            p_wig: OptString::new(),
            p_ww: OptString::new(),
            p_wc: std::cell::Cell::new(0),
            p_wcm: std::cell::Cell::new(0),
            p_wic: std::cell::Cell::new(false),
            p_wim: OptString::new(),
            p_wmnu: std::cell::Cell::new(false),
            p_winborder: OptString::new(),
            p_wh: std::cell::Cell::new(0),
            p_wmh: std::cell::Cell::new(0),
            p_wmw: std::cell::Cell::new(0),
            p_wiw: std::cell::Cell::new(0),
            p_wm: std::cell::Cell::new(0),
            p_ws: std::cell::Cell::new(false),
            p_write: std::cell::Cell::new(false),
            p_wa: std::cell::Cell::new(false),
            p_wb: std::cell::Cell::new(false),
            p_wd: std::cell::Cell::new(0),
            p_cdh: std::cell::Cell::new(false),
        }
    }
}

thread_local! {
    static OPT_VARS: &'static OptionVars = Box::leak(Box::new(OptionVars::new()));
}

#[inline(always)]
pub fn opt_vars() -> &'static OptionVars {
    OPT_VARS.with(|v| *v)
}

pub static BUF_OPT_IDX: [OptIndex; 93] = [
    6, 9, 10, 16, 21, 22, 27, 28, 29, 30, 35, 38, 39, 40, 41, 42, 48, 49, 51, 52, 54, 55, 60, 67,
    69, 71, 81, 82, 84, 87, 90, 92, 94, 97, 99, 100, 114, 115, 116, 117, 118, 119, 121, 122, 143,
    144, 146, 147, 149, 150, 151, 155, 159, 161, 172, 173, 174, 180, 181, 182, 192, 195, 196, 206,
    209, 219, 220, 232, 233, 247, 270, 283, 286, 288, 289, 290, 291, 300, 301, 303, 304, 308, 310,
    311, 314, 322, 323, 324, 336, 337, 341, 342, 373,
];
pub static OPT_AMBW_VALUES: [&std::ffi::CStr; 2] = [c"single", c"double"];
pub static OPT_BG_VALUES: [&std::ffi::CStr; 2] = [c"light", c"dark"];
pub static OPT_BS_VALUES: [&std::ffi::CStr; 4] = [c"indent", c"eol", c"start", c"nostop"];
pub static OPT_BKC_VALUES: [&std::ffi::CStr; 5] =
    [c"yes", c"auto", c"no", c"breaksymlink", c"breakhardlink"];
pub static OPT_BO_VALUES: [&std::ffi::CStr; 20] = [
    c"all",
    c"backspace",
    c"cursor",
    c"complete",
    c"copy",
    c"ctrlg",
    c"error",
    c"esc",
    c"ex",
    c"hangul",
    c"insertmode",
    c"lang",
    c"mess",
    c"showmatch",
    c"operator",
    c"register",
    c"shell",
    c"spell",
    c"term",
    c"wildmode",
];
pub static OPT_BRIOPT_VALUES: [&std::ffi::CStr; 5] =
    [c"shift:", c"min:", c"sbr", c"list:", c"column:"];
pub static OPT_BRIOPT_SCHEMA: [OptSchemaItem; 5] = [
    OptSchemaItem {
        name: c"shift",
        kind: OptSchemaKind::OptSchemaSNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"min",
        kind: OptSchemaKind::OptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"sbr",
        kind: OptSchemaKind::OptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"list",
        kind: OptSchemaKind::OptSchemaSNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"column",
        kind: OptSchemaKind::OptSchemaSNum,
        enum_values: &[],
    },
];
pub static OPT_BH_VALUES: [&std::ffi::CStr; 5] = [c"", c"hide", c"unload", c"delete", c"wipe"];
pub static OPT_BT_VALUES: [&std::ffi::CStr; 8] = [
    c"",
    c"acwrite",
    c"help",
    c"nofile",
    c"nowrite",
    c"quickfix",
    c"terminal",
    c"prompt",
];
pub static OPT_CMP_VALUES: [&std::ffi::CStr; 2] = [c"internal", c"keepascii"];
pub static OPT_CB_VALUES: [&std::ffi::CStr; 2] = [c"unnamed", c"unnamedplus"];
pub static OPT_CPT_VALUES: [&std::ffi::CStr; 15] = [
    c".", c"w", c"b", c"u", c"k", c"kspell", c"s", c"i", c"d", c"]", c"t", c"U", c"f", c"F", c"o",
];
pub static OPT_COT_VALUES: [&std::ffi::CStr; 13] = [
    c"fuzzy",
    c"longest",
    c"menu",
    c"menuone",
    c"nearest",
    c"noinsert",
    c"noselect",
    c"nosort",
    c"pathopt",
    c"popup",
    c"preinsert",
    c"preselect",
    c"preview",
];
pub static OPT_CSL_VALUES: [&std::ffi::CStr; 3] = [c"", c"slash", c"backslash"];
pub static OPT_CULOPT_VALUES: [&std::ffi::CStr; 4] = [c"line", c"screenline", c"number", c"both"];
pub static OPT_DEBUG_VALUES: [&std::ffi::CStr; 3] = [c"msg", c"throw", c"beep"];
pub static OPT_DIP_VALUES: [&std::ffi::CStr; 19] = [
    c"filler",
    c"anchor",
    c"context:",
    c"iblank",
    c"icase",
    c"iwhite",
    c"iwhiteall",
    c"iwhiteeol",
    c"horizontal",
    c"vertical",
    c"closeoff",
    c"hiddenoff",
    c"foldcolumn:",
    c"followwrap",
    c"internal",
    c"indent-heuristic",
    c"algorithm:",
    c"inline:",
    c"linematch:",
];
pub static OPT_DIP_ALGORITHM_VALUES: [&std::ffi::CStr; 4] =
    [c"myers", c"minimal", c"patience", c"histogram"];
pub static OPT_DIP_INLINE_VALUES: [&std::ffi::CStr; 4] = [c"none", c"simple", c"char", c"word"];
pub static OPT_DIP_SCHEMA: [OptSchemaItem; 19] = [
    OptSchemaItem {
        name: c"filler",
        kind: OptSchemaKind::OptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"anchor",
        kind: OptSchemaKind::OptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"context",
        kind: OptSchemaKind::OptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"iblank",
        kind: OptSchemaKind::OptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"icase",
        kind: OptSchemaKind::OptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"iwhite",
        kind: OptSchemaKind::OptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"iwhiteall",
        kind: OptSchemaKind::OptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"iwhiteeol",
        kind: OptSchemaKind::OptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"horizontal",
        kind: OptSchemaKind::OptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"vertical",
        kind: OptSchemaKind::OptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"closeoff",
        kind: OptSchemaKind::OptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"hiddenoff",
        kind: OptSchemaKind::OptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"foldcolumn",
        kind: OptSchemaKind::OptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"followwrap",
        kind: OptSchemaKind::OptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"internal",
        kind: OptSchemaKind::OptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"indent-heuristic",
        kind: OptSchemaKind::OptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"algorithm",
        kind: OptSchemaKind::OptSchemaEnum,
        enum_values: &OPT_DIP_ALGORITHM_VALUES,
    },
    OptSchemaItem {
        name: c"inline",
        kind: OptSchemaKind::OptSchemaEnum,
        enum_values: &OPT_DIP_INLINE_VALUES,
    },
    OptSchemaItem {
        name: c"linematch",
        kind: OptSchemaKind::OptSchemaNum,
        enum_values: &[],
    },
];
pub static OPT_DY_VALUES: [&std::ffi::CStr; 4] = [c"lastline", c"truncate", c"uhex", c"msgsep"];
pub static OPT_EAD_VALUES: [&std::ffi::CStr; 3] = [c"both", c"ver", c"hor"];
pub static OPT_FF_VALUES: [&std::ffi::CStr; 3] = [c"unix", c"dos", c"mac"];
pub static OPT_FCL_VALUES: [&std::ffi::CStr; 1] = [c"all"];
pub static OPT_FDC_VALUES: [&std::ffi::CStr; 20] = [
    c"auto", c"auto:1", c"auto:2", c"auto:3", c"auto:4", c"auto:5", c"auto:6", c"auto:7",
    c"auto:8", c"auto:9", c"0", c"1", c"2", c"3", c"4", c"5", c"6", c"7", c"8", c"9",
];
pub static OPT_FDM_VALUES: [&std::ffi::CStr; 6] =
    [c"manual", c"expr", c"marker", c"indent", c"syntax", c"diff"];
pub static OPT_FDO_VALUES: [&std::ffi::CStr; 11] = [
    c"all",
    c"block",
    c"hor",
    c"mark",
    c"percent",
    c"quickfix",
    c"search",
    c"tag",
    c"insert",
    c"undo",
    c"jump",
];
pub static OPT_ICM_VALUES: [&std::ffi::CStr; 3] = [c"nosplit", c"split", c""];
pub static OPT_JOP_VALUES: [&std::ffi::CStr; 3] = [c"stack", c"view", c"clean"];
pub static OPT_KM_VALUES: [&std::ffi::CStr; 2] = [c"startsel", c"stopsel"];
pub static OPT_LOP_VALUES: [&std::ffi::CStr; 2] = [c"expr:0", c"expr:1"];
pub static OPT_MOPT_VALUES: [&std::ffi::CStr; 7] = [
    c"hit-enter",
    c"history:",
    c"maxheight:",
    c"pager:",
    c"progress:",
    c"timeout:",
    c"wait:",
];
pub static OPT_MOPT_PROGRESS_VALUES: [&std::ffi::CStr; 2] = [c"", c"c"];
pub static OPT_MOPT_SCHEMA: [OptSchemaItem; 7] = [
    OptSchemaItem {
        name: c"hit-enter",
        kind: OptSchemaKind::OptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"history",
        kind: OptSchemaKind::OptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"maxheight",
        kind: OptSchemaKind::OptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"pager",
        kind: OptSchemaKind::OptSchemaStr,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"progress",
        kind: OptSchemaKind::OptSchemaEnum,
        enum_values: &OPT_MOPT_PROGRESS_VALUES,
    },
    OptSchemaItem {
        name: c"timeout",
        kind: OptSchemaKind::OptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"wait",
        kind: OptSchemaKind::OptSchemaNum,
        enum_values: &[],
    },
];
pub static OPT_MOUSEM_VALUES: [&std::ffi::CStr; 3] = [c"extend", c"popup", c"popup_setpos"];
pub static OPT_MOUSESCROLL_VALUES: [&std::ffi::CStr; 2] = [c"hor:", c"ver:"];
pub static OPT_MOUSESCROLL_SCHEMA: [OptSchemaItem; 2] = [
    OptSchemaItem {
        name: c"hor",
        kind: OptSchemaKind::OptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"ver",
        kind: OptSchemaKind::OptSchemaNum,
        enum_values: &[],
    },
];
pub static OPT_NF_VALUES: [&std::ffi::CStr; 6] =
    [c"bin", c"octal", c"hex", c"alpha", c"unsigned", c"blank"];
pub static OPT_PVP_VALUES: [&std::ffi::CStr; 3] = [c"height:", c"width:", c"border:"];
pub static OPT_PVP_BORDER_VALUES: [&std::ffi::CStr; 7] = [
    c"double", c"single", c"shadow", c"rounded", c"solid", c"bold", c"none",
];
pub static OPT_PVP_SCHEMA: [OptSchemaItem; 3] = [
    OptSchemaItem {
        name: c"height",
        kind: OptSchemaKind::OptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"width",
        kind: OptSchemaKind::OptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"border",
        kind: OptSchemaKind::OptSchemaEnum,
        enum_values: &OPT_PVP_BORDER_VALUES,
    },
];
pub static OPT_PUMBORDER_VALUES: [&std::ffi::CStr; 8] = [
    c"", c"double", c"single", c"shadow", c"rounded", c"solid", c"bold", c"none",
];
pub static OPT_RDB_VALUES: [&std::ffi::CStr; 6] = [
    c"compositor",
    c"nothrottle",
    c"invalid",
    c"nodelta",
    c"line",
    c"flush",
];
pub static OPT_RLC_VALUES: [&std::ffi::CStr; 1] = [c"search"];
pub static OPT_SBO_VALUES: [&std::ffi::CStr; 3] = [c"ver", c"hor", c"jump"];
pub static OPT_SEL_VALUES: [&std::ffi::CStr; 3] = [c"inclusive", c"exclusive", c"old"];
pub static OPT_SLM_VALUES: [&std::ffi::CStr; 3] = [c"mouse", c"key", c"cmd"];
pub static OPT_SSOP_VALUES: [&std::ffi::CStr; 18] = [
    c"buffers",
    c"winpos",
    c"resize",
    c"winsize",
    c"localoptions",
    c"options",
    c"help",
    c"blank",
    c"globals",
    c"slash",
    c"unix",
    c"sesdir",
    c"curdir",
    c"folds",
    c"cursor",
    c"tabpages",
    c"terminal",
    c"skiprtp",
];
pub static OPT_SLOC_VALUES: [&std::ffi::CStr; 3] = [c"last", c"statusline", c"tabline"];
pub static OPT_SCL_VALUES: [&std::ffi::CStr; 22] = [
    c"yes", c"no", c"auto", c"auto:1", c"auto:2", c"auto:3", c"auto:4", c"auto:5", c"auto:6",
    c"auto:7", c"auto:8", c"auto:9", c"yes:1", c"yes:2", c"yes:3", c"yes:4", c"yes:5", c"yes:6",
    c"yes:7", c"yes:8", c"yes:9", c"number",
];
pub static OPT_SPO_VALUES: [&std::ffi::CStr; 2] = [c"camel", c"noplainbuffer"];
pub static OPT_SPS_VALUES: [&std::ffi::CStr; 6] =
    [c"best", c"fast", c"double", c"expr:", c"file:", c"timeout:"];
pub static OPT_SPK_VALUES: [&std::ffi::CStr; 3] = [c"cursor", c"screen", c"topline"];
pub static OPT_SWB_VALUES: [&std::ffi::CStr; 6] = [
    c"useopen", c"usetab", c"split", c"newtab", c"vsplit", c"uselast",
];
pub static OPT_TCL_VALUES: [&std::ffi::CStr; 2] = [c"left", c"uselast"];
pub static OPT_TC_VALUES: [&std::ffi::CStr; 5] =
    [c"followic", c"ignore", c"match", c"followscs", c"smart"];
pub static OPT_TPF_VALUES: [&std::ffi::CStr; 7] =
    [c"BS", c"HT", c"FF", c"ESC", c"DEL", c"C0", c"C1"];
pub static OPT_VE_VALUES: [&std::ffi::CStr; 6] =
    [c"block", c"insert", c"all", c"onemore", c"none", c"NONE"];
pub static OPT_WIM_VALUES: [&std::ffi::CStr; 6] = [
    c"full",
    c"longest",
    c"list",
    c"lastused",
    c"noselect",
    c"noinsert",
];
pub static OPT_WOP_VALUES: [&std::ffi::CStr; 4] = [c"fuzzy", c"tagfile", c"pum", c"exacttext"];
pub static OPT_WAK_VALUES: [&std::ffi::CStr; 3] = [c"yes", c"menu", c"no"];
pub static OPT_WINBORDER_VALUES: [&std::ffi::CStr; 8] = [
    c"", c"double", c"single", c"shadow", c"rounded", c"solid", c"bold", c"none",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_vars_defaults_match_c_initializers() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(opt_vars().dy_escape_width.get(), 2);
        assert_eq!(opt_vars().p_mousescroll_vert.get(), 3);
        assert_eq!(opt_vars().p_mousescroll_hor.get(), 6);
        assert!(opt_vars().p_vfile.is_empty());
    }
}
