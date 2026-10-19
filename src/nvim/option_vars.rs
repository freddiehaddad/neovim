//! Global variables for settable options (`src/nvim/option_vars.h`).
#![allow(non_upper_case_globals)]

use std::ffi::{c_char, c_int, c_uint};
use std::mem::zeroed;
use std::ptr;

use crate::eval::deprecated::Callback;
use crate::option::OptInt;

#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut empty_string_option: [c_char; 1] = [0 as c_char; 1];
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ambw: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_acd: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ai: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_bin: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_bomb: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_bl: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cin: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_channel: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cink: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cinsd: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cinw: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cfu: Callback = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ofu: Callback = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tsrfu: Callback = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ci: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ar: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_aw: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_awa: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_bs: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_bg: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_bk: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_bkc: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut bkc_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_bdir: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_bex: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_bo: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut breakat_flags: [c_char; 256] = [0 as c_char; 256];
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut bo_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_bsk: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_breakat: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_bh: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_bt: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_busy: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cmp: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut cmp_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_enc: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_deco: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ccv: Callback = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cino: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cedit: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cb: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut cb_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cwh: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ch: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cms: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cpt: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cto: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_columns: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_confirm: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cia: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut cia_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cot: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut cot_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ac: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_act: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_acl: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_csl: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_pumborder: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_pb: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ph: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_pw: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_pmw: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_com: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cpo: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_debug: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_def: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_inc: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_dia: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_dip: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_dex: Callback = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_dict: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_dg: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_dir: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_dy: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut dy_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut dy_escape_width: c_int = 2;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ead: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_emoji: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ea: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ep: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_eb: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ef: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_efm: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_gefm: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_gp: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_eof: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_eol: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ei: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_et: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_exrc: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_fenc: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_fencs: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ff: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ffs: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_fic: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ft: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_fcs: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ffu: Callback = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_fixeol: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_fcl: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_fdls: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_fdo: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut fdo_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_follow: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_fex: Callback = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_flp: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_fo: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_fp: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_fs: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_gd: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_guicursor: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_guifont: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_guifontwide: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_hf: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_hh: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_hlg: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_hid: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_hl: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_hls: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_hi: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_arshape: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_icon: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_iconstring: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ic: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_iminsert: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_imsearch: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_inf: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_inex: Callback = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_is: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_inde: Callback = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_indk: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_icm: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_isf: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_isi: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_isk: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_js: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_jop: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut jop_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_keymap: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_kp: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_km: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_langmap: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_lnr: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_lrm: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_lm: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_lines: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_linespace: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_lisp: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_lop: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_lispwords: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ls: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_stal: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_lcs: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_lz: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_lpl: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_magic: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_menc: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mef: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mp: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mps: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mat: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mco: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mfd: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mmd: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mmp: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mis: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mopt: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_msc: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_msm: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ml: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mle: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mls: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ma: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mod: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mouse: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mousem: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mousemev: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mousef: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mh: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mousescroll: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mousescroll_vert: OptInt = 3;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mousescroll_hor: OptInt = 6;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_mouset: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_more: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_nf: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_opfunc: Callback = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_para: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_paste: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_pex: Callback = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_pm: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_path: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cdpath: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_pi: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_pyx: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_qe: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ro: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_rdb: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut rdb_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_rdt: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_re: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_report: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_pvp: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_pvh: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_chi: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ari: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ri: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ru: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ruf: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_plf: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_pp: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_qftf: Callback = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_rtp: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_scbk: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sj: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_so: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sop: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sbo: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sections: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_secure: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sel: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_slm: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ssop: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut ssop_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sh: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_shcf: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sp: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_shq: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sxq: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sxe: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_srr: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_stmp: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ssl: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_stl: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_wbr: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sr: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sw: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_shm: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sbr: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sc: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sloc: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sft: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sm: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_smd: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ss: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_siso: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_scs: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_si: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sta: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sts: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sb: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sua: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_swf: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_smc: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tpm: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tal: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tpf: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut tpf_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tfu: Callback = unsafe { zeroed() };
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_spc: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_spf: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_spl: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_spo: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut spo_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sps: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_spr: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_sol: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_su: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_swb: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut swb_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_spk: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_syn: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tcl: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut tcl_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ts: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tbs: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tc: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut tc_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tl: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tr: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tags: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tgst: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tbidi: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tw: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_to: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_timeout: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tm: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_title: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_titlelen: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_titleold: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_titlestring: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tsr: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tgc: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ttimeout: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ttm: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_tf: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_udir: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_udf: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ul: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ur: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_uc: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ut: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_shada: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_shadafile: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_termsync: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_vsts: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_vts: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_vdir: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_vop: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut vop_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_vb: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ve: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut ve_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_verbose: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_warn: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_wop: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut wop_flags: c_uint = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_window: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_wak: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_wig: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ww: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_wc: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_wcm: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_wic: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_wim: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_wmnu: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_winborder: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_wh: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_wmh: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_wmw: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_wiw: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_wm: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_ws: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_write: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_wa: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_wb: c_int = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_wd: OptInt = 0;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut p_cdh: c_int = 0;

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
        }
    }
}
