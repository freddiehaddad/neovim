//! Global variables for settable options (`src/nvim/option_vars.h`).

use std::ffi::{c_char, c_int, c_uint};
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
pub static mut p_cfu: Callback = Callback::NONE;
pub static mut p_ofu: Callback = Callback::NONE;
pub static mut p_tsrfu: Callback = Callback::NONE;
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
pub static mut p_ccv: Callback = Callback::NONE;
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
pub static mut p_dex: Callback = Callback::NONE;
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
pub static mut p_fic: c_int = 0;
pub static mut p_ft: *mut c_char = ptr::null_mut();
pub static mut p_fcs: *mut c_char = ptr::null_mut();
pub static mut p_ffu: Callback = Callback::NONE;
pub static mut p_fixeol: c_int = 0;
pub static mut p_fcl: *mut c_char = ptr::null_mut();
pub static mut p_fdls: OptInt = 0;
pub static mut p_fdo: *mut c_char = ptr::null_mut();
pub static mut fdo_flags: c_uint = 0;
pub static mut p_follow: c_int = 0;
pub static mut p_fex: Callback = Callback::NONE;
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
pub static mut p_arshape: c_int = 0;
pub static mut p_icon: c_int = 0;
pub static mut p_iconstring: *mut c_char = ptr::null_mut();
pub static mut p_ic: c_int = 0;
pub static mut p_iminsert: OptInt = 0;
pub static mut p_imsearch: OptInt = 0;
pub static mut p_inf: c_int = 0;
pub static mut p_inex: Callback = Callback::NONE;
pub static mut p_is: c_int = 0;
pub static mut p_inde: Callback = Callback::NONE;
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
pub static mut p_opfunc: Callback = Callback::NONE;
pub static mut p_para: *mut c_char = ptr::null_mut();
pub static mut p_paste: c_int = 0;
pub static mut p_pex: Callback = Callback::NONE;
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
pub static mut p_qftf: Callback = Callback::NONE;
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
pub static mut p_sh: *mut c_char = ptr::null_mut();
pub static mut p_shcf: *mut c_char = ptr::null_mut();
pub static mut p_sp: *mut c_char = ptr::null_mut();
pub static mut p_shq: *mut c_char = ptr::null_mut();
pub static mut p_sxq: *mut c_char = ptr::null_mut();
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
pub static mut p_tfu: Callback = Callback::NONE;
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
pub static opt_ambw_values: [&std::ffi::CStr; 2] = [c"single", c"double"];
pub static opt_bg_values: [&std::ffi::CStr; 2] = [c"light", c"dark"];
pub static opt_bs_values: [&std::ffi::CStr; 4] = [
    c"indent",
    c"eol",
    c"start",
    c"nostop"];
pub static opt_bkc_values: [&std::ffi::CStr; 5] = [
    c"yes",
    c"auto",
    c"no",
    c"breaksymlink",
    c"breakhardlink"];
pub static opt_bo_values: [&std::ffi::CStr; 20] = [
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
    c"wildmode"];
pub static opt_briopt_values: [&std::ffi::CStr; 5] = [
    c"shift:",
    c"min:",
    c"sbr",
    c"list:",
    c"column:"];
pub static opt_briopt_schema: [OptSchemaItem; 5] = [
    OptSchemaItem {
        name: c"shift",
        kind: OptSchemaKind::kOptSchemaSNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"min",
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"sbr",
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"list",
        kind: OptSchemaKind::kOptSchemaSNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"column",
        kind: OptSchemaKind::kOptSchemaSNum,
        enum_values: &[],
    },
];
pub static opt_bh_values: [&std::ffi::CStr; 5] = [
    c"",
    c"hide",
    c"unload",
    c"delete",
    c"wipe"];
pub static opt_bt_values: [&std::ffi::CStr; 8] = [
    c"",
    c"acwrite",
    c"help",
    c"nofile",
    c"nowrite",
    c"quickfix",
    c"terminal",
    c"prompt"];
pub static opt_cmp_values: [&std::ffi::CStr; 2] = [c"internal", c"keepascii"];
pub static opt_cb_values: [&std::ffi::CStr; 2] = [c"unnamed", c"unnamedplus"];
pub static opt_cpt_values: [&std::ffi::CStr; 15] = [
    c".",
    c"w",
    c"b",
    c"u",
    c"k",
    c"kspell",
    c"s",
    c"i",
    c"d",
    c"]",
    c"t",
    c"U",
    c"f",
    c"F",
    c"o"];
pub static opt_cot_values: [&std::ffi::CStr; 13] = [
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
    c"preview"];
pub static opt_csl_values: [&std::ffi::CStr; 3] = [
    c"",
    c"slash",
    c"backslash"];
pub static opt_culopt_values: [&std::ffi::CStr; 4] = [
    c"line",
    c"screenline",
    c"number",
    c"both"];
pub static opt_debug_values: [&std::ffi::CStr; 3] = [
    c"msg",
    c"throw",
    c"beep"];
pub static opt_dip_values: [&std::ffi::CStr; 19] = [
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
    c"linematch:"];
pub static opt_dip_algorithm_values: [&std::ffi::CStr; 4] = [
    c"myers",
    c"minimal",
    c"patience",
    c"histogram"];
pub static opt_dip_inline_values: [&std::ffi::CStr; 4] = [
    c"none",
    c"simple",
    c"char",
    c"word"];
pub static opt_dip_schema: [OptSchemaItem; 19] = [
    OptSchemaItem {
        name: c"filler",
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"anchor",
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"context",
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"iblank",
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"icase",
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"iwhite",
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"iwhiteall",
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"iwhiteeol",
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"horizontal",
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"vertical",
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"closeoff",
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"hiddenoff",
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"foldcolumn",
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"followwrap",
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"internal",
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"indent-heuristic",
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"algorithm",
        kind: OptSchemaKind::kOptSchemaEnum,
        enum_values: &opt_dip_algorithm_values,
    },
    OptSchemaItem {
        name: c"inline",
        kind: OptSchemaKind::kOptSchemaEnum,
        enum_values: &opt_dip_inline_values,
    },
    OptSchemaItem {
        name: c"linematch",
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: &[],
    },
];
pub static opt_dy_values: [&std::ffi::CStr; 4] = [
    c"lastline",
    c"truncate",
    c"uhex",
    c"msgsep"];
pub static opt_ead_values: [&std::ffi::CStr; 3] = [
    c"both",
    c"ver",
    c"hor"];
pub static opt_ff_values: [&std::ffi::CStr; 3] = [
    c"unix",
    c"dos",
    c"mac"];
pub static opt_fcl_values: [&std::ffi::CStr; 1] = [c"all"];
pub static opt_fdc_values: [&std::ffi::CStr; 20] = [
    c"auto",
    c"auto:1",
    c"auto:2",
    c"auto:3",
    c"auto:4",
    c"auto:5",
    c"auto:6",
    c"auto:7",
    c"auto:8",
    c"auto:9",
    c"0",
    c"1",
    c"2",
    c"3",
    c"4",
    c"5",
    c"6",
    c"7",
    c"8",
    c"9"];
pub static opt_fdm_values: [&std::ffi::CStr; 6] = [
    c"manual",
    c"expr",
    c"marker",
    c"indent",
    c"syntax",
    c"diff"];
pub static opt_fdo_values: [&std::ffi::CStr; 11] = [
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
    c"jump"];
pub static opt_icm_values: [&std::ffi::CStr; 3] = [
    c"nosplit",
    c"split",
    c""];
pub static opt_jop_values: [&std::ffi::CStr; 3] = [
    c"stack",
    c"view",
    c"clean"];
pub static opt_km_values: [&std::ffi::CStr; 2] = [c"startsel", c"stopsel"];
pub static opt_lop_values: [&std::ffi::CStr; 2] = [c"expr:0", c"expr:1"];
pub static opt_mopt_values: [&std::ffi::CStr; 7] = [
    c"hit-enter",
    c"history:",
    c"maxheight:",
    c"pager:",
    c"progress:",
    c"timeout:",
    c"wait:"];
pub static opt_mopt_progress_values: [&std::ffi::CStr; 2] = [c"", c"c"];
pub static opt_mopt_schema: [OptSchemaItem; 7] = [
    OptSchemaItem {
        name: c"hit-enter",
        kind: OptSchemaKind::kOptSchemaFlag,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"history",
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"maxheight",
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"pager",
        kind: OptSchemaKind::kOptSchemaStr,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"progress",
        kind: OptSchemaKind::kOptSchemaEnum,
        enum_values: &opt_mopt_progress_values,
    },
    OptSchemaItem {
        name: c"timeout",
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"wait",
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: &[],
    },
];
pub static opt_mousem_values: [&std::ffi::CStr; 3] = [
    c"extend",
    c"popup",
    c"popup_setpos"];
pub static opt_mousescroll_values: [&std::ffi::CStr; 2] = [c"hor:", c"ver:"];
pub static opt_mousescroll_schema: [OptSchemaItem; 2] = [
    OptSchemaItem {
        name: c"hor",
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"ver",
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: &[],
    },
];
pub static opt_nf_values: [&std::ffi::CStr; 6] = [
    c"bin",
    c"octal",
    c"hex",
    c"alpha",
    c"unsigned",
    c"blank"];
pub static opt_pvp_values: [&std::ffi::CStr; 3] = [
    c"height:",
    c"width:",
    c"border:"];
pub static opt_pvp_border_values: [&std::ffi::CStr; 7] = [
    c"double",
    c"single",
    c"shadow",
    c"rounded",
    c"solid",
    c"bold",
    c"none"];
pub static opt_pvp_schema: [OptSchemaItem; 3] = [
    OptSchemaItem {
        name: c"height",
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"width",
        kind: OptSchemaKind::kOptSchemaNum,
        enum_values: &[],
    },
    OptSchemaItem {
        name: c"border",
        kind: OptSchemaKind::kOptSchemaEnum,
        enum_values: &opt_pvp_border_values,
    },
];
pub static opt_pumborder_values: [&std::ffi::CStr; 8] = [
    c"",
    c"double",
    c"single",
    c"shadow",
    c"rounded",
    c"solid",
    c"bold",
    c"none"];
pub static opt_rdb_values: [&std::ffi::CStr; 6] = [
    c"compositor",
    c"nothrottle",
    c"invalid",
    c"nodelta",
    c"line",
    c"flush"];
pub static opt_rlc_values: [&std::ffi::CStr; 1] = [c"search"];
pub static opt_sbo_values: [&std::ffi::CStr; 3] = [
    c"ver",
    c"hor",
    c"jump"];
pub static opt_sel_values: [&std::ffi::CStr; 3] = [
    c"inclusive",
    c"exclusive",
    c"old"];
pub static opt_slm_values: [&std::ffi::CStr; 3] = [
    c"mouse",
    c"key",
    c"cmd"];
pub static opt_ssop_values: [&std::ffi::CStr; 18] = [
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
    c"skiprtp"];
pub static opt_sloc_values: [&std::ffi::CStr; 3] = [
    c"last",
    c"statusline",
    c"tabline"];
pub static opt_scl_values: [&std::ffi::CStr; 22] = [
    c"yes",
    c"no",
    c"auto",
    c"auto:1",
    c"auto:2",
    c"auto:3",
    c"auto:4",
    c"auto:5",
    c"auto:6",
    c"auto:7",
    c"auto:8",
    c"auto:9",
    c"yes:1",
    c"yes:2",
    c"yes:3",
    c"yes:4",
    c"yes:5",
    c"yes:6",
    c"yes:7",
    c"yes:8",
    c"yes:9",
    c"number"];
pub static opt_spo_values: [&std::ffi::CStr; 2] = [c"camel", c"noplainbuffer"];
pub static opt_sps_values: [&std::ffi::CStr; 6] = [
    c"best",
    c"fast",
    c"double",
    c"expr:",
    c"file:",
    c"timeout:"];
pub static opt_spk_values: [&std::ffi::CStr; 3] = [
    c"cursor",
    c"screen",
    c"topline"];
pub static opt_swb_values: [&std::ffi::CStr; 6] = [
    c"useopen",
    c"usetab",
    c"split",
    c"newtab",
    c"vsplit",
    c"uselast"];
pub static opt_tcl_values: [&std::ffi::CStr; 2] = [c"left", c"uselast"];
pub static opt_tc_values: [&std::ffi::CStr; 5] = [
    c"followic",
    c"ignore",
    c"match",
    c"followscs",
    c"smart"];
pub static opt_tpf_values: [&std::ffi::CStr; 7] = [
    c"BS",
    c"HT",
    c"FF",
    c"ESC",
    c"DEL",
    c"C0",
    c"C1"];
pub static opt_ve_values: [&std::ffi::CStr; 6] = [
    c"block",
    c"insert",
    c"all",
    c"onemore",
    c"none",
    c"NONE"];
pub static opt_wim_values: [&std::ffi::CStr; 6] = [
    c"full",
    c"longest",
    c"list",
    c"lastused",
    c"noselect",
    c"noinsert"];
pub static opt_wop_values: [&std::ffi::CStr; 4] = [
    c"fuzzy",
    c"tagfile",
    c"pum",
    c"exacttext"];
pub static opt_wak_values: [&std::ffi::CStr; 3] = [
    c"yes",
    c"menu",
    c"no"];
pub static opt_winborder_values: [&std::ffi::CStr; 8] = [
    c"",
    c"double",
    c"single",
    c"shadow",
    c"rounded",
    c"solid",
    c"bold",
    c"none"];

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
