//! Global variables for settable options (`src/nvim/option_vars.h`).

use std::ffi::{c_char, c_int, c_uint};
use std::mem::zeroed;
use std::ptr;

use crate::eval::typval::Callback;
use crate::option::OptInt;
use crate::optionstr::{OptIndex, OptSchemaItem, OptSchemaKind};

pub static mut empty_string_option: [c_char; 1] = [0 as c_char; 1];
pub static mut p_ambw: *mut c_char = ptr::null_mut();
pub static mut p_acd: c_int = 0;
pub static mut p_ai: c_int = 0;
pub static mut p_bin: c_int = 0;
pub static mut p_bomb: c_int = 0;
pub static mut p_bl: c_int = 0;
pub static mut p_cin: c_int = 0;
pub static mut p_channel: OptInt = 0;
pub static mut p_cink: *mut c_char = ptr::null_mut();
pub static mut p_cinsd: *mut c_char = ptr::null_mut();
pub static mut p_cinw: *mut c_char = ptr::null_mut();
pub static mut p_cfu: Callback = unsafe { zeroed() };
pub static mut p_ofu: Callback = unsafe { zeroed() };
pub static mut p_tsrfu: Callback = unsafe { zeroed() };
pub static mut p_ci: c_int = 0;
pub static mut p_ar: c_int = 0;
pub static mut p_aw: c_int = 0;
pub static mut p_awa: c_int = 0;
pub static mut p_bs: *mut c_char = ptr::null_mut();
pub static mut p_bg: *mut c_char = ptr::null_mut();
pub static mut p_bk: c_int = 0;
pub static mut p_bkc: *mut c_char = ptr::null_mut();
pub static mut bkc_flags: c_uint = 0;
pub static mut p_bdir: *mut c_char = ptr::null_mut();
pub static mut p_bex: *mut c_char = ptr::null_mut();
pub static mut p_bo: *mut c_char = ptr::null_mut();
pub static mut breakat_flags: [c_char; 256] = [0 as c_char; 256];
pub static mut bo_flags: c_uint = 0;
pub static mut p_bsk: *mut c_char = ptr::null_mut();
pub static mut p_breakat: *mut c_char = ptr::null_mut();
pub static mut p_bh: *mut c_char = ptr::null_mut();
pub static mut p_bt: *mut c_char = ptr::null_mut();
pub static mut p_busy: OptInt = 0;
pub static mut p_cmp: *mut c_char = ptr::null_mut();
pub static mut cmp_flags: c_uint = 0;
pub static mut p_enc: *mut c_char = ptr::null_mut();
pub static mut p_deco: c_int = 0;
pub static mut p_ccv: Callback = unsafe { zeroed() };
pub static mut p_cino: *mut c_char = ptr::null_mut();
pub static mut p_cedit: *mut c_char = ptr::null_mut();
pub static mut p_cb: *mut c_char = ptr::null_mut();
pub static mut cb_flags: c_uint = 0;
pub static mut p_cwh: OptInt = 0;
pub static mut p_ch: OptInt = 0;
pub static mut p_cms: *mut c_char = ptr::null_mut();
pub static mut p_cpt: *mut c_char = ptr::null_mut();
pub static mut p_cto: OptInt = 0;
pub static mut p_columns: OptInt = 0;
pub static mut p_confirm: c_int = 0;
pub static mut p_cia: *mut c_char = ptr::null_mut();
pub static mut cia_flags: c_uint = 0;
pub static mut p_cot: *mut c_char = ptr::null_mut();
pub static mut cot_flags: c_uint = 0;
pub static mut p_ac: c_int = 0;
pub static mut p_act: OptInt = 0;
pub static mut p_acl: OptInt = 0;
#[cfg(windows)]
pub static mut p_csl: *mut c_char = ptr::null_mut();
pub static mut p_pumborder: *mut c_char = ptr::null_mut();
pub static mut p_pb: OptInt = 0;
pub static mut p_ph: OptInt = 0;
pub static mut p_pw: OptInt = 0;
pub static mut p_pmw: OptInt = 0;
pub static mut p_com: *mut c_char = ptr::null_mut();
pub static mut p_cpo: *mut c_char = ptr::null_mut();
pub static mut p_debug: *mut c_char = ptr::null_mut();
pub static mut p_def: *mut c_char = ptr::null_mut();
pub static mut p_inc: *mut c_char = ptr::null_mut();
pub static mut p_dia: *mut c_char = ptr::null_mut();
pub static mut p_dip: *mut c_char = ptr::null_mut();
pub static mut p_dex: Callback = unsafe { zeroed() };
pub static mut p_dict: *mut c_char = ptr::null_mut();
pub static mut p_dg: c_int = 0;
pub static mut p_dir: *mut c_char = ptr::null_mut();
pub static mut p_dy: *mut c_char = ptr::null_mut();
pub static mut dy_flags: c_uint = 0;
pub static mut dy_escape_width: c_int = 2;
pub static mut p_ead: *mut c_char = ptr::null_mut();
pub static mut p_emoji: c_int = 0;
pub static mut p_ea: c_int = 0;
pub static mut p_ep: *mut c_char = ptr::null_mut();
pub static mut p_eb: c_int = 0;
pub static mut p_ef: *mut c_char = ptr::null_mut();
pub static mut p_efm: *mut c_char = ptr::null_mut();
pub static mut p_gefm: *mut c_char = ptr::null_mut();
pub static mut p_gp: *mut c_char = ptr::null_mut();
pub static mut p_eof: c_int = 0;
pub static mut p_eol: c_int = 0;
pub static mut p_ei: *mut c_char = ptr::null_mut();
pub static mut p_et: c_int = 0;
pub static mut p_exrc: c_int = 0;
pub static mut p_fenc: *mut c_char = ptr::null_mut();
pub static mut p_fencs: *mut c_char = ptr::null_mut();
pub static mut p_ff: *mut c_char = ptr::null_mut();
pub static mut p_ffs: *mut c_char = ptr::null_mut();
#[unsafe(no_mangle)]
pub static mut p_fic: c_int = 0;
pub static mut p_ft: *mut c_char = ptr::null_mut();
pub static mut p_fcs: *mut c_char = ptr::null_mut();
pub static mut p_ffu: Callback = unsafe { zeroed() };
pub static mut p_fixeol: c_int = 0;
pub static mut p_fcl: *mut c_char = ptr::null_mut();
pub static mut p_fdls: OptInt = 0;
pub static mut p_fdo: *mut c_char = ptr::null_mut();
pub static mut fdo_flags: c_uint = 0;
pub static mut p_follow: c_int = 0;
pub static mut p_fex: Callback = unsafe { zeroed() };
pub static mut p_flp: *mut c_char = ptr::null_mut();
pub static mut p_fo: *mut c_char = ptr::null_mut();
pub static mut p_fp: *mut c_char = ptr::null_mut();
pub static mut p_fs: c_int = 0;
pub static mut p_gd: c_int = 0;
pub static mut p_guicursor: *mut c_char = ptr::null_mut();
pub static mut p_guifont: *mut c_char = ptr::null_mut();
pub static mut p_guifontwide: *mut c_char = ptr::null_mut();
pub static mut p_hf: *mut c_char = ptr::null_mut();
pub static mut p_hh: OptInt = 0;
pub static mut p_hlg: *mut c_char = ptr::null_mut();
pub static mut p_hid: c_int = 0;
pub static mut p_hl: *mut c_char = ptr::null_mut();
pub static mut p_hls: c_int = 0;
pub static mut p_hi: OptInt = 0;
#[unsafe(no_mangle)]
pub static mut p_arshape: c_int = 0;
pub static mut p_icon: c_int = 0;
pub static mut p_iconstring: *mut c_char = ptr::null_mut();
pub static mut p_ic: c_int = 0;
pub static mut p_iminsert: OptInt = 0;
pub static mut p_imsearch: OptInt = 0;
pub static mut p_inf: c_int = 0;
pub static mut p_inex: Callback = unsafe { zeroed() };
pub static mut p_is: c_int = 0;
pub static mut p_inde: Callback = unsafe { zeroed() };
pub static mut p_indk: *mut c_char = ptr::null_mut();
pub static mut p_icm: *mut c_char = ptr::null_mut();
pub static mut p_isf: *mut c_char = ptr::null_mut();
pub static mut p_isi: *mut c_char = ptr::null_mut();
pub static mut p_isk: *mut c_char = ptr::null_mut();
pub static mut p_js: c_int = 0;
pub static mut p_jop: *mut c_char = ptr::null_mut();
pub static mut jop_flags: c_uint = 0;
pub static mut p_keymap: *mut c_char = ptr::null_mut();
pub static mut p_kp: *mut c_char = ptr::null_mut();
pub static mut p_km: *mut c_char = ptr::null_mut();
pub static mut p_langmap: *mut c_char = ptr::null_mut();
pub static mut p_lnr: c_int = 0;
pub static mut p_lrm: c_int = 0;
pub static mut p_lm: *mut c_char = ptr::null_mut();
pub static mut p_lines: OptInt = 0;
pub static mut p_linespace: OptInt = 0;
pub static mut p_lisp: c_int = 0;
pub static mut p_lop: *mut c_char = ptr::null_mut();
pub static mut p_lispwords: *mut c_char = ptr::null_mut();
pub static mut p_ls: OptInt = 0;
pub static mut p_stal: OptInt = 0;
pub static mut p_lcs: *mut c_char = ptr::null_mut();
pub static mut p_lz: c_int = 0;
pub static mut p_lpl: c_int = 0;
pub static mut p_magic: c_int = 0;
pub static mut p_menc: *mut c_char = ptr::null_mut();
pub static mut p_mef: *mut c_char = ptr::null_mut();
pub static mut p_mp: *mut c_char = ptr::null_mut();
pub static mut p_mps: *mut c_char = ptr::null_mut();
pub static mut p_mat: OptInt = 0;
pub static mut p_mco: OptInt = 0;
pub static mut p_mfd: OptInt = 0;
pub static mut p_mmd: OptInt = 0;
pub static mut p_mmp: OptInt = 0;
pub static mut p_mis: OptInt = 0;
pub static mut p_mopt: *mut c_char = ptr::null_mut();
pub static mut p_msc: OptInt = 0;
pub static mut p_msm: *mut c_char = ptr::null_mut();
pub static mut p_ml: c_int = 0;
pub static mut p_mle: c_int = 0;
pub static mut p_mls: OptInt = 0;
pub static mut p_ma: c_int = 0;
pub static mut p_mod: c_int = 0;
pub static mut p_mouse: *mut c_char = ptr::null_mut();
pub static mut p_mousem: *mut c_char = ptr::null_mut();
pub static mut p_mousemev: c_int = 0;
pub static mut p_mousef: c_int = 0;
pub static mut p_mh: c_int = 0;
pub static mut p_mousescroll: *mut c_char = ptr::null_mut();
pub static mut p_mousescroll_vert: OptInt = 3;
pub static mut p_mousescroll_hor: OptInt = 6;
pub static mut p_mouset: OptInt = 0;
pub static mut p_more: c_int = 0;
pub static mut p_nf: *mut c_char = ptr::null_mut();
pub static mut p_opfunc: Callback = unsafe { zeroed() };
pub static mut p_para: *mut c_char = ptr::null_mut();
pub static mut p_paste: c_int = 0;
pub static mut p_pex: Callback = unsafe { zeroed() };
pub static mut p_pm: *mut c_char = ptr::null_mut();
pub static mut p_path: *mut c_char = ptr::null_mut();
pub static mut p_cdpath: *mut c_char = ptr::null_mut();
pub static mut p_pi: c_int = 0;
pub static mut p_pyx: OptInt = 0;
pub static mut p_qe: *mut c_char = ptr::null_mut();
pub static mut p_ro: c_int = 0;
pub static mut p_rdb: *mut c_char = ptr::null_mut();
pub static mut rdb_flags: c_uint = 0;
pub static mut p_rdt: OptInt = 0;
pub static mut p_re: OptInt = 0;
pub static mut p_report: OptInt = 0;
pub static mut p_pvp: *mut c_char = ptr::null_mut();
pub static mut p_pvh: OptInt = 0;
pub static mut p_chi: OptInt = 0;
pub static mut p_ari: c_int = 0;
pub static mut p_ri: c_int = 0;
pub static mut p_ru: c_int = 0;
pub static mut p_ruf: *mut c_char = ptr::null_mut();
pub static mut p_plf: *mut c_char = ptr::null_mut();
pub static mut p_pp: *mut c_char = ptr::null_mut();
pub static mut p_qftf: Callback = unsafe { zeroed() };
pub static mut p_rtp: *mut c_char = ptr::null_mut();
pub static mut p_scbk: OptInt = 0;
pub static mut p_sj: OptInt = 0;
pub static mut p_so: OptInt = 0;
pub static mut p_sop: OptInt = 0;
pub static mut p_sbo: *mut c_char = ptr::null_mut();
pub static mut p_sections: *mut c_char = ptr::null_mut();
pub static mut p_secure: c_int = 0;
pub static mut p_sel: *mut c_char = ptr::null_mut();
pub static mut p_slm: *mut c_char = ptr::null_mut();
pub static mut p_ssop: *mut c_char = ptr::null_mut();
pub static mut ssop_flags: c_uint = 0;
#[unsafe(no_mangle)]
pub static mut p_sh: *mut c_char = ptr::null_mut();
#[unsafe(no_mangle)]
pub static mut p_shcf: *mut c_char = ptr::null_mut();
pub static mut p_sp: *mut c_char = ptr::null_mut();
pub static mut p_shq: *mut c_char = ptr::null_mut();
#[unsafe(no_mangle)]
pub static mut p_sxq: *mut c_char = ptr::null_mut();
#[unsafe(no_mangle)]
pub static mut p_sxe: *mut c_char = ptr::null_mut();
pub static mut p_srr: *mut c_char = ptr::null_mut();
pub static mut p_stmp: c_int = 0;
#[cfg(windows)]
pub static mut p_ssl: c_int = 0;
pub static mut p_stl: *mut c_char = ptr::null_mut();
pub static mut p_wbr: *mut c_char = ptr::null_mut();
pub static mut p_sr: c_int = 0;
pub static mut p_sw: OptInt = 0;
pub static mut p_shm: *mut c_char = ptr::null_mut();
pub static mut p_sbr: *mut c_char = ptr::null_mut();
pub static mut p_sc: c_int = 0;
pub static mut p_sloc: *mut c_char = ptr::null_mut();
pub static mut p_sft: c_int = 0;
pub static mut p_sm: c_int = 0;
pub static mut p_smd: c_int = 0;
pub static mut p_ss: OptInt = 0;
pub static mut p_siso: OptInt = 0;
pub static mut p_scs: c_int = 0;
pub static mut p_si: c_int = 0;
pub static mut p_sta: c_int = 0;
pub static mut p_sts: OptInt = 0;
pub static mut p_sb: c_int = 0;
pub static mut p_sua: *mut c_char = ptr::null_mut();
pub static mut p_swf: c_int = 0;
pub static mut p_smc: OptInt = 0;
pub static mut p_tpm: OptInt = 0;
pub static mut p_tal: *mut c_char = ptr::null_mut();
pub static mut p_tpf: *mut c_char = ptr::null_mut();
pub static mut tpf_flags: c_uint = 0;
pub static mut p_tfu: Callback = unsafe { zeroed() };
pub static mut p_spc: *mut c_char = ptr::null_mut();
pub static mut p_spf: *mut c_char = ptr::null_mut();
pub static mut p_spl: *mut c_char = ptr::null_mut();
pub static mut p_spo: *mut c_char = ptr::null_mut();
pub static mut spo_flags: c_uint = 0;
pub static mut p_sps: *mut c_char = ptr::null_mut();
pub static mut p_spr: c_int = 0;
pub static mut p_sol: c_int = 0;
pub static mut p_su: *mut c_char = ptr::null_mut();
pub static mut p_swb: *mut c_char = ptr::null_mut();
pub static mut swb_flags: c_uint = 0;
pub static mut p_spk: *mut c_char = ptr::null_mut();
pub static mut p_syn: *mut c_char = ptr::null_mut();
pub static mut p_tcl: *mut c_char = ptr::null_mut();
pub static mut tcl_flags: c_uint = 0;
pub static mut p_ts: OptInt = 0;
pub static mut p_tbs: c_int = 0;
pub static mut p_tc: *mut c_char = ptr::null_mut();
pub static mut tc_flags: c_uint = 0;
pub static mut p_tl: OptInt = 0;
pub static mut p_tr: c_int = 0;
pub static mut p_tags: *mut c_char = ptr::null_mut();
pub static mut p_tgst: c_int = 0;
#[unsafe(no_mangle)]
pub static mut p_tbidi: c_int = 0;
pub static mut p_tw: OptInt = 0;
pub static mut p_to: c_int = 0;
pub static mut p_timeout: c_int = 0;
pub static mut p_tm: OptInt = 0;
pub static mut p_title: c_int = 0;
pub static mut p_titlelen: OptInt = 0;
pub static mut p_titleold: *mut c_char = ptr::null_mut();
pub static mut p_titlestring: *mut c_char = ptr::null_mut();
pub static mut p_tsr: *mut c_char = ptr::null_mut();
pub static mut p_tgc: c_int = 0;
pub static mut p_ttimeout: c_int = 0;
pub static mut p_ttm: OptInt = 0;
pub static mut p_tf: c_int = 0;
#[unsafe(no_mangle)]
pub static mut p_udir: *mut c_char = ptr::null_mut();
pub static mut p_udf: c_int = 0;
pub static mut p_ul: OptInt = 0;
pub static mut p_ur: OptInt = 0;
pub static mut p_uc: OptInt = 0;
pub static mut p_ut: OptInt = 0;
pub static mut p_shada: *mut c_char = ptr::null_mut();
pub static mut p_shadafile: *mut c_char = ptr::null_mut();
pub static mut p_termsync: c_int = 0;
pub static mut p_vsts: *mut c_char = ptr::null_mut();
pub static mut p_vts: *mut c_char = ptr::null_mut();
pub static mut p_vdir: *mut c_char = ptr::null_mut();
pub static mut p_vop: *mut c_char = ptr::null_mut();
pub static mut vop_flags: c_uint = 0;
pub static mut p_vb: c_int = 0;
pub static mut p_ve: *mut c_char = ptr::null_mut();
pub static mut ve_flags: c_uint = 0;
pub static mut p_verbose: OptInt = 0;
pub static mut p_vfile: *mut c_char = (&raw mut empty_string_option).cast();
pub static mut p_warn: c_int = 0;
pub static mut p_wop: *mut c_char = ptr::null_mut();
pub static mut wop_flags: c_uint = 0;
pub static mut p_window: OptInt = 0;
pub static mut p_wak: *mut c_char = ptr::null_mut();
pub static mut p_wig: *mut c_char = ptr::null_mut();
pub static mut p_ww: *mut c_char = ptr::null_mut();
pub static mut p_wc: OptInt = 0;
pub static mut p_wcm: OptInt = 0;
pub static mut p_wic: c_int = 0;
pub static mut p_wim: *mut c_char = ptr::null_mut();
pub static mut p_wmnu: c_int = 0;
pub static mut p_winborder: *mut c_char = ptr::null_mut();
pub static mut p_wh: OptInt = 0;
pub static mut p_wmh: OptInt = 0;
pub static mut p_wmw: OptInt = 0;
pub static mut p_wiw: OptInt = 0;
pub static mut p_wm: OptInt = 0;
pub static mut p_ws: c_int = 0;
pub static mut p_write: c_int = 0;
pub static mut p_wa: c_int = 0;
pub static mut p_wb: c_int = 0;
pub static mut p_wd: OptInt = 0;
pub static mut p_cdh: c_int = 0;

pub static buf_opt_idx: [OptIndex; 93] = [
    6, 9, 10, 16, 21, 22, 27, 28, 29, 30, 35, 38, 39, 40, 41, 42, 48, 49, 51, 52, 54, 55, 60, 67,
    69, 71, 81, 82, 84, 87, 90, 92, 94, 97, 99, 100, 114, 115, 116, 117, 118, 119, 121, 122, 143,
    144, 146, 147, 149, 150, 151, 155, 159, 161, 172, 173, 174, 180, 181, 182, 192, 195, 196, 206,
    209, 219, 220, 232, 233, 247, 270, 283, 286, 288, 289, 290, 291, 300, 301, 303, 304, 308, 310,
    311, 314, 322, 323, 324, 336, 337, 341, 342, 373,
];
pub static mut opt_ambw_values: [*const c_char; 3] =
    [c"single".as_ptr(), c"double".as_ptr(), ptr::null()];
pub static mut opt_bg_values: [*const c_char; 3] =
    [c"light".as_ptr(), c"dark".as_ptr(), ptr::null()];
pub static mut opt_bs_values: [*const c_char; 5] = [
    c"indent".as_ptr(),
    c"eol".as_ptr(),
    c"start".as_ptr(),
    c"nostop".as_ptr(),
    ptr::null(),
];
pub static mut opt_bkc_values: [*const c_char; 6] = [
    c"yes".as_ptr(),
    c"auto".as_ptr(),
    c"no".as_ptr(),
    c"breaksymlink".as_ptr(),
    c"breakhardlink".as_ptr(),
    ptr::null(),
];
pub static mut opt_bo_values: [*const c_char; 21] = [
    c"all".as_ptr(),
    c"backspace".as_ptr(),
    c"cursor".as_ptr(),
    c"complete".as_ptr(),
    c"copy".as_ptr(),
    c"ctrlg".as_ptr(),
    c"error".as_ptr(),
    c"esc".as_ptr(),
    c"ex".as_ptr(),
    c"hangul".as_ptr(),
    c"insertmode".as_ptr(),
    c"lang".as_ptr(),
    c"mess".as_ptr(),
    c"showmatch".as_ptr(),
    c"operator".as_ptr(),
    c"register".as_ptr(),
    c"shell".as_ptr(),
    c"spell".as_ptr(),
    c"term".as_ptr(),
    c"wildmode".as_ptr(),
    ptr::null(),
];
pub static mut opt_briopt_values: [*const c_char; 6] = [
    c"shift:".as_ptr(),
    c"min:".as_ptr(),
    c"sbr".as_ptr(),
    c"list:".as_ptr(),
    c"column:".as_ptr(),
    ptr::null(),
];
pub static opt_briopt_schema: [OptSchemaItem; 6] = [
    OptSchemaItem {
        name: c"shift".as_ptr(),
        kind: OptSchemaKind::kOptSchemaSNum,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"min".as_ptr(),
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"sbr".as_ptr(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"list".as_ptr(),
        kind: OptSchemaKind::kOptSchemaSNum,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"column".as_ptr(),
        kind: OptSchemaKind::kOptSchemaSNum,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: ptr::null(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
];
pub static mut opt_bh_values: [*const c_char; 6] = [
    c"".as_ptr(),
    c"hide".as_ptr(),
    c"unload".as_ptr(),
    c"delete".as_ptr(),
    c"wipe".as_ptr(),
    ptr::null(),
];
pub static mut opt_bt_values: [*const c_char; 9] = [
    c"".as_ptr(),
    c"acwrite".as_ptr(),
    c"help".as_ptr(),
    c"nofile".as_ptr(),
    c"nowrite".as_ptr(),
    c"quickfix".as_ptr(),
    c"terminal".as_ptr(),
    c"prompt".as_ptr(),
    ptr::null(),
];
pub static mut opt_cmp_values: [*const c_char; 3] =
    [c"internal".as_ptr(), c"keepascii".as_ptr(), ptr::null()];
pub static mut opt_cb_values: [*const c_char; 3] =
    [c"unnamed".as_ptr(), c"unnamedplus".as_ptr(), ptr::null()];
pub static mut opt_cpt_values: [*const c_char; 16] = [
    c".".as_ptr(),
    c"w".as_ptr(),
    c"b".as_ptr(),
    c"u".as_ptr(),
    c"k".as_ptr(),
    c"kspell".as_ptr(),
    c"s".as_ptr(),
    c"i".as_ptr(),
    c"d".as_ptr(),
    c"]".as_ptr(),
    c"t".as_ptr(),
    c"U".as_ptr(),
    c"f".as_ptr(),
    c"F".as_ptr(),
    c"o".as_ptr(),
    ptr::null(),
];
pub static mut opt_cot_values: [*const c_char; 13] = [
    c"fuzzy".as_ptr(),
    c"longest".as_ptr(),
    c"menu".as_ptr(),
    c"menuone".as_ptr(),
    c"nearest".as_ptr(),
    c"noinsert".as_ptr(),
    c"noselect".as_ptr(),
    c"nosort".as_ptr(),
    c"popup".as_ptr(),
    c"preinsert".as_ptr(),
    c"preselect".as_ptr(),
    c"preview".as_ptr(),
    ptr::null(),
];
pub static mut opt_csl_values: [*const c_char; 4] = [
    c"".as_ptr(),
    c"slash".as_ptr(),
    c"backslash".as_ptr(),
    ptr::null(),
];
pub static mut opt_culopt_values: [*const c_char; 5] = [
    c"line".as_ptr(),
    c"screenline".as_ptr(),
    c"number".as_ptr(),
    c"both".as_ptr(),
    ptr::null(),
];
pub static mut opt_debug_values: [*const c_char; 4] = [
    c"msg".as_ptr(),
    c"throw".as_ptr(),
    c"beep".as_ptr(),
    ptr::null(),
];
pub static mut opt_dip_values: [*const c_char; 20] = [
    c"filler".as_ptr(),
    c"anchor".as_ptr(),
    c"context:".as_ptr(),
    c"iblank".as_ptr(),
    c"icase".as_ptr(),
    c"iwhite".as_ptr(),
    c"iwhiteall".as_ptr(),
    c"iwhiteeol".as_ptr(),
    c"horizontal".as_ptr(),
    c"vertical".as_ptr(),
    c"closeoff".as_ptr(),
    c"hiddenoff".as_ptr(),
    c"foldcolumn:".as_ptr(),
    c"followwrap".as_ptr(),
    c"internal".as_ptr(),
    c"indent-heuristic".as_ptr(),
    c"algorithm:".as_ptr(),
    c"inline:".as_ptr(),
    c"linematch:".as_ptr(),
    ptr::null(),
];
pub static mut opt_dip_algorithm_values: [*const c_char; 5] = [
    c"myers".as_ptr(),
    c"minimal".as_ptr(),
    c"patience".as_ptr(),
    c"histogram".as_ptr(),
    ptr::null(),
];
pub static mut opt_dip_inline_values: [*const c_char; 5] = [
    c"none".as_ptr(),
    c"simple".as_ptr(),
    c"char".as_ptr(),
    c"word".as_ptr(),
    ptr::null(),
];
pub static opt_dip_schema: [OptSchemaItem; 20] = [
    OptSchemaItem {
        name: c"filler".as_ptr(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"anchor".as_ptr(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"context".as_ptr(),
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"iblank".as_ptr(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"icase".as_ptr(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"iwhite".as_ptr(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"iwhiteall".as_ptr(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"iwhiteeol".as_ptr(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"horizontal".as_ptr(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"vertical".as_ptr(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"closeoff".as_ptr(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"hiddenoff".as_ptr(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"foldcolumn".as_ptr(),
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"followwrap".as_ptr(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"internal".as_ptr(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"indent-heuristic".as_ptr(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"algorithm".as_ptr(),
        kind: OptSchemaKind::kOptSchemaEnum,
        enum_values: (&raw const opt_dip_algorithm_values).cast(),
    },
    OptSchemaItem {
        name: c"inline".as_ptr(),
        kind: OptSchemaKind::kOptSchemaEnum,
        enum_values: (&raw const opt_dip_inline_values).cast(),
    },
    OptSchemaItem {
        name: c"linematch".as_ptr(),
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: ptr::null(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
];
pub static mut opt_dy_values: [*const c_char; 5] = [
    c"lastline".as_ptr(),
    c"truncate".as_ptr(),
    c"uhex".as_ptr(),
    c"msgsep".as_ptr(),
    ptr::null(),
];
pub static mut opt_ead_values: [*const c_char; 4] = [
    c"both".as_ptr(),
    c"ver".as_ptr(),
    c"hor".as_ptr(),
    ptr::null(),
];
pub static mut opt_ff_values: [*const c_char; 4] = [
    c"unix".as_ptr(),
    c"dos".as_ptr(),
    c"mac".as_ptr(),
    ptr::null(),
];
pub static mut opt_fcl_values: [*const c_char; 2] = [c"all".as_ptr(), ptr::null()];
pub static mut opt_fdc_values: [*const c_char; 21] = [
    c"auto".as_ptr(),
    c"auto:1".as_ptr(),
    c"auto:2".as_ptr(),
    c"auto:3".as_ptr(),
    c"auto:4".as_ptr(),
    c"auto:5".as_ptr(),
    c"auto:6".as_ptr(),
    c"auto:7".as_ptr(),
    c"auto:8".as_ptr(),
    c"auto:9".as_ptr(),
    c"0".as_ptr(),
    c"1".as_ptr(),
    c"2".as_ptr(),
    c"3".as_ptr(),
    c"4".as_ptr(),
    c"5".as_ptr(),
    c"6".as_ptr(),
    c"7".as_ptr(),
    c"8".as_ptr(),
    c"9".as_ptr(),
    ptr::null(),
];
pub static mut opt_fdm_values: [*const c_char; 7] = [
    c"manual".as_ptr(),
    c"expr".as_ptr(),
    c"marker".as_ptr(),
    c"indent".as_ptr(),
    c"syntax".as_ptr(),
    c"diff".as_ptr(),
    ptr::null(),
];
pub static mut opt_fdo_values: [*const c_char; 12] = [
    c"all".as_ptr(),
    c"block".as_ptr(),
    c"hor".as_ptr(),
    c"mark".as_ptr(),
    c"percent".as_ptr(),
    c"quickfix".as_ptr(),
    c"search".as_ptr(),
    c"tag".as_ptr(),
    c"insert".as_ptr(),
    c"undo".as_ptr(),
    c"jump".as_ptr(),
    ptr::null(),
];
pub static mut opt_icm_values: [*const c_char; 4] = [
    c"nosplit".as_ptr(),
    c"split".as_ptr(),
    c"".as_ptr(),
    ptr::null(),
];
pub static mut opt_jop_values: [*const c_char; 4] = [
    c"stack".as_ptr(),
    c"view".as_ptr(),
    c"clean".as_ptr(),
    ptr::null(),
];
pub static mut opt_km_values: [*const c_char; 3] =
    [c"startsel".as_ptr(), c"stopsel".as_ptr(), ptr::null()];
pub static mut opt_lop_values: [*const c_char; 3] =
    [c"expr:0".as_ptr(), c"expr:1".as_ptr(), ptr::null()];
pub static mut opt_mopt_values: [*const c_char; 8] = [
    c"hit-enter".as_ptr(),
    c"history:".as_ptr(),
    c"maxheight:".as_ptr(),
    c"pager:".as_ptr(),
    c"progress:".as_ptr(),
    c"timeout:".as_ptr(),
    c"wait:".as_ptr(),
    ptr::null(),
];
pub static mut opt_mopt_progress_values: [*const c_char; 3] =
    [c"".as_ptr(), c"c".as_ptr(), ptr::null()];
pub static opt_mopt_schema: [OptSchemaItem; 8] = [
    OptSchemaItem {
        name: c"hit-enter".as_ptr(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"history".as_ptr(),
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"maxheight".as_ptr(),
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"pager".as_ptr(),
        kind: OptSchemaKind::kOptSchemaStr,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"progress".as_ptr(),
        kind: OptSchemaKind::kOptSchemaEnum,
        enum_values: (&raw const opt_mopt_progress_values).cast(),
    },
    OptSchemaItem {
        name: c"timeout".as_ptr(),
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"wait".as_ptr(),
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: ptr::null(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
];
pub static mut opt_mousem_values: [*const c_char; 4] = [
    c"extend".as_ptr(),
    c"popup".as_ptr(),
    c"popup_setpos".as_ptr(),
    ptr::null(),
];
pub static mut opt_mousescroll_values: [*const c_char; 3] =
    [c"hor:".as_ptr(), c"ver:".as_ptr(), ptr::null()];
pub static opt_mousescroll_schema: [OptSchemaItem; 3] = [
    OptSchemaItem {
        name: c"hor".as_ptr(),
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"ver".as_ptr(),
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: ptr::null(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
];
pub static mut opt_nf_values: [*const c_char; 7] = [
    c"bin".as_ptr(),
    c"octal".as_ptr(),
    c"hex".as_ptr(),
    c"alpha".as_ptr(),
    c"unsigned".as_ptr(),
    c"blank".as_ptr(),
    ptr::null(),
];
pub static mut opt_pvp_values: [*const c_char; 4] = [
    c"height:".as_ptr(),
    c"width:".as_ptr(),
    c"border:".as_ptr(),
    ptr::null(),
];
pub static mut opt_pvp_border_values: [*const c_char; 8] = [
    c"double".as_ptr(),
    c"single".as_ptr(),
    c"shadow".as_ptr(),
    c"rounded".as_ptr(),
    c"solid".as_ptr(),
    c"bold".as_ptr(),
    c"none".as_ptr(),
    ptr::null(),
];
pub static opt_pvp_schema: [OptSchemaItem; 4] = [
    OptSchemaItem {
        name: c"height".as_ptr(),
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"width".as_ptr(),
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: ptr::null(),
    },
    OptSchemaItem {
        name: c"border".as_ptr(),
        kind: OptSchemaKind::kOptSchemaEnum,
        enum_values: (&raw const opt_pvp_border_values).cast(),
    },
    OptSchemaItem {
        name: ptr::null(),
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: ptr::null(),
    },
];
pub static mut opt_pumborder_values: [*const c_char; 9] = [
    c"".as_ptr(),
    c"double".as_ptr(),
    c"single".as_ptr(),
    c"shadow".as_ptr(),
    c"rounded".as_ptr(),
    c"solid".as_ptr(),
    c"bold".as_ptr(),
    c"none".as_ptr(),
    ptr::null(),
];
pub static mut opt_rdb_values: [*const c_char; 7] = [
    c"compositor".as_ptr(),
    c"nothrottle".as_ptr(),
    c"invalid".as_ptr(),
    c"nodelta".as_ptr(),
    c"line".as_ptr(),
    c"flush".as_ptr(),
    ptr::null(),
];
pub static mut opt_rlc_values: [*const c_char; 2] = [c"search".as_ptr(), ptr::null()];
pub static mut opt_sbo_values: [*const c_char; 4] = [
    c"ver".as_ptr(),
    c"hor".as_ptr(),
    c"jump".as_ptr(),
    ptr::null(),
];
pub static mut opt_sel_values: [*const c_char; 4] = [
    c"inclusive".as_ptr(),
    c"exclusive".as_ptr(),
    c"old".as_ptr(),
    ptr::null(),
];
pub static mut opt_slm_values: [*const c_char; 4] = [
    c"mouse".as_ptr(),
    c"key".as_ptr(),
    c"cmd".as_ptr(),
    ptr::null(),
];
pub static mut opt_ssop_values: [*const c_char; 19] = [
    c"buffers".as_ptr(),
    c"winpos".as_ptr(),
    c"resize".as_ptr(),
    c"winsize".as_ptr(),
    c"localoptions".as_ptr(),
    c"options".as_ptr(),
    c"help".as_ptr(),
    c"blank".as_ptr(),
    c"globals".as_ptr(),
    c"slash".as_ptr(),
    c"unix".as_ptr(),
    c"sesdir".as_ptr(),
    c"curdir".as_ptr(),
    c"folds".as_ptr(),
    c"cursor".as_ptr(),
    c"tabpages".as_ptr(),
    c"terminal".as_ptr(),
    c"skiprtp".as_ptr(),
    ptr::null(),
];
pub static mut opt_sloc_values: [*const c_char; 4] = [
    c"last".as_ptr(),
    c"statusline".as_ptr(),
    c"tabline".as_ptr(),
    ptr::null(),
];
pub static mut opt_scl_values: [*const c_char; 23] = [
    c"yes".as_ptr(),
    c"no".as_ptr(),
    c"auto".as_ptr(),
    c"auto:1".as_ptr(),
    c"auto:2".as_ptr(),
    c"auto:3".as_ptr(),
    c"auto:4".as_ptr(),
    c"auto:5".as_ptr(),
    c"auto:6".as_ptr(),
    c"auto:7".as_ptr(),
    c"auto:8".as_ptr(),
    c"auto:9".as_ptr(),
    c"yes:1".as_ptr(),
    c"yes:2".as_ptr(),
    c"yes:3".as_ptr(),
    c"yes:4".as_ptr(),
    c"yes:5".as_ptr(),
    c"yes:6".as_ptr(),
    c"yes:7".as_ptr(),
    c"yes:8".as_ptr(),
    c"yes:9".as_ptr(),
    c"number".as_ptr(),
    ptr::null(),
];
pub static mut opt_spo_values: [*const c_char; 3] =
    [c"camel".as_ptr(), c"noplainbuffer".as_ptr(), ptr::null()];
pub static mut opt_sps_values: [*const c_char; 7] = [
    c"best".as_ptr(),
    c"fast".as_ptr(),
    c"double".as_ptr(),
    c"expr:".as_ptr(),
    c"file:".as_ptr(),
    c"timeout:".as_ptr(),
    ptr::null(),
];
pub static mut opt_spk_values: [*const c_char; 4] = [
    c"cursor".as_ptr(),
    c"screen".as_ptr(),
    c"topline".as_ptr(),
    ptr::null(),
];
pub static mut opt_swb_values: [*const c_char; 7] = [
    c"useopen".as_ptr(),
    c"usetab".as_ptr(),
    c"split".as_ptr(),
    c"newtab".as_ptr(),
    c"vsplit".as_ptr(),
    c"uselast".as_ptr(),
    ptr::null(),
];
pub static mut opt_tcl_values: [*const c_char; 3] =
    [c"left".as_ptr(), c"uselast".as_ptr(), ptr::null()];
pub static mut opt_tc_values: [*const c_char; 6] = [
    c"followic".as_ptr(),
    c"ignore".as_ptr(),
    c"match".as_ptr(),
    c"followscs".as_ptr(),
    c"smart".as_ptr(),
    ptr::null(),
];
pub static mut opt_tpf_values: [*const c_char; 8] = [
    c"BS".as_ptr(),
    c"HT".as_ptr(),
    c"FF".as_ptr(),
    c"ESC".as_ptr(),
    c"DEL".as_ptr(),
    c"C0".as_ptr(),
    c"C1".as_ptr(),
    ptr::null(),
];
pub static mut opt_ve_values: [*const c_char; 7] = [
    c"block".as_ptr(),
    c"insert".as_ptr(),
    c"all".as_ptr(),
    c"onemore".as_ptr(),
    c"none".as_ptr(),
    c"NONE".as_ptr(),
    ptr::null(),
];
pub static mut opt_wim_values: [*const c_char; 7] = [
    c"full".as_ptr(),
    c"longest".as_ptr(),
    c"list".as_ptr(),
    c"lastused".as_ptr(),
    c"noselect".as_ptr(),
    c"noinsert".as_ptr(),
    ptr::null(),
];
pub static mut opt_wop_values: [*const c_char; 5] = [
    c"fuzzy".as_ptr(),
    c"tagfile".as_ptr(),
    c"pum".as_ptr(),
    c"exacttext".as_ptr(),
    ptr::null(),
];
pub static mut opt_wak_values: [*const c_char; 4] = [
    c"yes".as_ptr(),
    c"menu".as_ptr(),
    c"no".as_ptr(),
    ptr::null(),
];
pub static mut opt_winborder_values: [*const c_char; 9] = [
    c"".as_ptr(),
    c"double".as_ptr(),
    c"single".as_ptr(),
    c"shadow".as_ptr(),
    c"rounded".as_ptr(),
    c"solid".as_ptr(),
    c"bold".as_ptr(),
    c"none".as_ptr(),
    ptr::null(),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_vars_defaults_match_c_initializers() {
        unsafe {
            assert_eq!({ dy_escape_width }, 2);
            assert_eq!({ p_mousescroll_vert }, 3);
            assert_eq!({ p_mousescroll_hor }, 6);
            assert_eq!({ empty_string_option[0] }, 0);
            assert_eq!({ *p_vfile }, 0);
        }
    }
}
