//! Functions for Arabic language shaping (`src/nvim/arabic.c`).

use crate::option_vars::*;
use std::ffi::c_int;

#[inline]
fn arshape_enabled() -> bool {
    opt_vars().p_arshape != 0 && opt_vars().p_tbidi == 0
}

const A_HAMZA: u32 = 0x0621;
const A_ALEF_MADDA: u32 = 0x0622;
const A_ALEF_HAMZA_ABOVE: u32 = 0x0623;
const A_WAW_HAMZA: u32 = 0x0624;
const A_ALEF_HAMZA_BELOW: u32 = 0x0625;
const A_YEH_HAMZA: u32 = 0x0626;
const A_ALEF: u32 = 0x0627;
const A_BEH: u32 = 0x0628;
const A_TEH_MARBUTA: u32 = 0x0629;
const A_TEH: u32 = 0x062a;
const A_THEH: u32 = 0x062b;
const A_JEEM: u32 = 0x062c;
const A_HAH: u32 = 0x062d;
const A_KHAH: u32 = 0x062e;
const A_DAL: u32 = 0x062f;
const A_THAL: u32 = 0x0630;
const A_REH: u32 = 0x0631;
const A_ZAIN: u32 = 0x0632;
const A_SEEN: u32 = 0x0633;
const A_SHEEN: u32 = 0x0634;
const A_SAD: u32 = 0x0635;
const A_DAD: u32 = 0x0636;
const A_TAH: u32 = 0x0637;
const A_ZAH: u32 = 0x0638;
const A_AIN: u32 = 0x0639;
const A_GHAIN: u32 = 0x063a;
const A_TATWEEL: u32 = 0x0640;
const A_FEH: u32 = 0x0641;
const A_QAF: u32 = 0x0642;
const A_KAF: u32 = 0x0643;
const A_LAM: u32 = 0x0644;
const A_MEEM: u32 = 0x0645;
const A_NOON: u32 = 0x0646;
const A_HEH: u32 = 0x0647;
const A_WAW: u32 = 0x0648;
const A_ALEF_MAKSURA: u32 = 0x0649;
const A_YEH: u32 = 0x064a;
const A_FATHATAN: u32 = 0x064b;
const A_DAMMATAN: u32 = 0x064c;
const A_KASRATAN: u32 = 0x064d;
const A_FATHA: u32 = 0x064e;
const A_DAMMA: u32 = 0x064f;
const A_KASRA: u32 = 0x0650;
const A_SHADDA: u32 = 0x0651;
const A_SUKUN: u32 = 0x0652;
const A_MADDA_ABOVE: u32 = 0x0653;
const A_HAMZA_ABOVE: u32 = 0x0654;
const A_HAMZA_BELOW: u32 = 0x0655;

const A_PEH: u32 = 0x067e;
const A_TCHEH: u32 = 0x0686;
const A_JEH: u32 = 0x0698;
const A_FKAF: u32 = 0x06a9;
const A_GAF: u32 = 0x06af;
const A_FYEH: u32 = 0x06cc;

const A_S_LAM_ALEF_MADDA_ABOVE: c_int = 0xfef5;
const A_F_LAM_ALEF_MADDA_ABOVE: c_int = 0xfef6;
const A_S_LAM_ALEF_HAMZA_ABOVE: c_int = 0xfef7;
const A_F_LAM_ALEF_HAMZA_ABOVE: c_int = 0xfef8;
const A_S_LAM_ALEF_HAMZA_BELOW: c_int = 0xfef9;
const A_F_LAM_ALEF_HAMZA_BELOW: c_int = 0xfefa;
const A_S_LAM_ALEF: c_int = 0xfefb;
const A_F_LAM_ALEF: c_int = 0xfefc;

const A_BYTE_ORDER_MARK: u32 = 0xfeff;

#[derive(Clone, Copy)]
struct AChar {
    c: u32,
    isolated: u32,
    initial: u32,
    medial: u32,
    final_form: u32,
}

const ACHARS: [AChar; 54] = [
    AChar {
        c: A_HAMZA,
        isolated: 0xfe80,
        initial: 0,
        medial: 0,
        final_form: 0,
    },
    AChar {
        c: A_ALEF_MADDA,
        isolated: 0xfe81,
        initial: 0,
        medial: 0,
        final_form: 0xfe82,
    },
    AChar {
        c: A_ALEF_HAMZA_ABOVE,
        isolated: 0xfe83,
        initial: 0,
        medial: 0,
        final_form: 0xfe84,
    },
    AChar {
        c: A_WAW_HAMZA,
        isolated: 0xfe85,
        initial: 0,
        medial: 0,
        final_form: 0xfe86,
    },
    AChar {
        c: A_ALEF_HAMZA_BELOW,
        isolated: 0xfe87,
        initial: 0,
        medial: 0,
        final_form: 0xfe88,
    },
    AChar {
        c: A_YEH_HAMZA,
        isolated: 0xfe89,
        initial: 0xfe8b,
        medial: 0xfe8c,
        final_form: 0xfe8a,
    },
    AChar {
        c: A_ALEF,
        isolated: 0xfe8d,
        initial: 0,
        medial: 0,
        final_form: 0xfe8e,
    },
    AChar {
        c: A_BEH,
        isolated: 0xfe8f,
        initial: 0xfe91,
        medial: 0xfe92,
        final_form: 0xfe90,
    },
    AChar {
        c: A_TEH_MARBUTA,
        isolated: 0xfe93,
        initial: 0,
        medial: 0,
        final_form: 0xfe94,
    },
    AChar {
        c: A_TEH,
        isolated: 0xfe95,
        initial: 0xfe97,
        medial: 0xfe98,
        final_form: 0xfe96,
    },
    AChar {
        c: A_THEH,
        isolated: 0xfe99,
        initial: 0xfe9b,
        medial: 0xfe9c,
        final_form: 0xfe9a,
    },
    AChar {
        c: A_JEEM,
        isolated: 0xfe9d,
        initial: 0xfe9f,
        medial: 0xfea0,
        final_form: 0xfe9e,
    },
    AChar {
        c: A_HAH,
        isolated: 0xfea1,
        initial: 0xfea3,
        medial: 0xfea4,
        final_form: 0xfea2,
    },
    AChar {
        c: A_KHAH,
        isolated: 0xfea5,
        initial: 0xfea7,
        medial: 0xfea8,
        final_form: 0xfea6,
    },
    AChar {
        c: A_DAL,
        isolated: 0xfea9,
        initial: 0,
        medial: 0,
        final_form: 0xfeaa,
    },
    AChar {
        c: A_THAL,
        isolated: 0xfeab,
        initial: 0,
        medial: 0,
        final_form: 0xfeac,
    },
    AChar {
        c: A_REH,
        isolated: 0xfead,
        initial: 0,
        medial: 0,
        final_form: 0xfeae,
    },
    AChar {
        c: A_ZAIN,
        isolated: 0xfeaf,
        initial: 0,
        medial: 0,
        final_form: 0xfeb0,
    },
    AChar {
        c: A_SEEN,
        isolated: 0xfeb1,
        initial: 0xfeb3,
        medial: 0xfeb4,
        final_form: 0xfeb2,
    },
    AChar {
        c: A_SHEEN,
        isolated: 0xfeb5,
        initial: 0xfeb7,
        medial: 0xfeb8,
        final_form: 0xfeb6,
    },
    AChar {
        c: A_SAD,
        isolated: 0xfeb9,
        initial: 0xfebb,
        medial: 0xfebc,
        final_form: 0xfeba,
    },
    AChar {
        c: A_DAD,
        isolated: 0xfebd,
        initial: 0xfebf,
        medial: 0xfec0,
        final_form: 0xfebe,
    },
    AChar {
        c: A_TAH,
        isolated: 0xfec1,
        initial: 0xfec3,
        medial: 0xfec4,
        final_form: 0xfec2,
    },
    AChar {
        c: A_ZAH,
        isolated: 0xfec5,
        initial: 0xfec7,
        medial: 0xfec8,
        final_form: 0xfec6,
    },
    AChar {
        c: A_AIN,
        isolated: 0xfec9,
        initial: 0xfecb,
        medial: 0xfecc,
        final_form: 0xfeca,
    },
    AChar {
        c: A_GHAIN,
        isolated: 0xfecd,
        initial: 0xfecf,
        medial: 0xfed0,
        final_form: 0xfece,
    },
    AChar {
        c: A_TATWEEL,
        isolated: 0,
        initial: 0x0640,
        medial: 0x0640,
        final_form: 0x0640,
    },
    AChar {
        c: A_FEH,
        isolated: 0xfed1,
        initial: 0xfed3,
        medial: 0xfed4,
        final_form: 0xfed2,
    },
    AChar {
        c: A_QAF,
        isolated: 0xfed5,
        initial: 0xfed7,
        medial: 0xfed8,
        final_form: 0xfed6,
    },
    AChar {
        c: A_KAF,
        isolated: 0xfed9,
        initial: 0xfedb,
        medial: 0xfedc,
        final_form: 0xfeda,
    },
    AChar {
        c: A_LAM,
        isolated: 0xfedd,
        initial: 0xfedf,
        medial: 0xfee0,
        final_form: 0xfede,
    },
    AChar {
        c: A_MEEM,
        isolated: 0xfee1,
        initial: 0xfee3,
        medial: 0xfee4,
        final_form: 0xfee2,
    },
    AChar {
        c: A_NOON,
        isolated: 0xfee5,
        initial: 0xfee7,
        medial: 0xfee8,
        final_form: 0xfee6,
    },
    AChar {
        c: A_HEH,
        isolated: 0xfee9,
        initial: 0xfeeb,
        medial: 0xfeec,
        final_form: 0xfeea,
    },
    AChar {
        c: A_WAW,
        isolated: 0xfeed,
        initial: 0,
        medial: 0,
        final_form: 0xfeee,
    },
    AChar {
        c: A_ALEF_MAKSURA,
        isolated: 0xfeef,
        initial: 0,
        medial: 0,
        final_form: 0xfef0,
    },
    AChar {
        c: A_YEH,
        isolated: 0xfef1,
        initial: 0xfef3,
        medial: 0xfef4,
        final_form: 0xfef2,
    },
    AChar {
        c: A_FATHATAN,
        isolated: 0xfe70,
        initial: 0,
        medial: 0,
        final_form: 0,
    },
    AChar {
        c: A_DAMMATAN,
        isolated: 0xfe72,
        initial: 0,
        medial: 0,
        final_form: 0,
    },
    AChar {
        c: A_KASRATAN,
        isolated: 0xfe74,
        initial: 0,
        medial: 0,
        final_form: 0,
    },
    AChar {
        c: A_FATHA,
        isolated: 0xfe76,
        initial: 0,
        medial: 0xfe77,
        final_form: 0,
    },
    AChar {
        c: A_DAMMA,
        isolated: 0xfe78,
        initial: 0,
        medial: 0xfe79,
        final_form: 0,
    },
    AChar {
        c: A_KASRA,
        isolated: 0xfe7a,
        initial: 0,
        medial: 0xfe7b,
        final_form: 0,
    },
    AChar {
        c: A_SHADDA,
        isolated: 0xfe7c,
        initial: 0,
        medial: 0xfe7c,
        final_form: 0,
    },
    AChar {
        c: A_SUKUN,
        isolated: 0xfe7e,
        initial: 0,
        medial: 0xfe7f,
        final_form: 0,
    },
    AChar {
        c: A_MADDA_ABOVE,
        isolated: 0,
        initial: 0,
        medial: 0,
        final_form: 0,
    },
    AChar {
        c: A_HAMZA_ABOVE,
        isolated: 0,
        initial: 0,
        medial: 0,
        final_form: 0,
    },
    AChar {
        c: A_HAMZA_BELOW,
        isolated: 0,
        initial: 0,
        medial: 0,
        final_form: 0,
    },
    AChar {
        c: A_PEH,
        isolated: 0xfb56,
        initial: 0xfb58,
        medial: 0xfb59,
        final_form: 0xfb57,
    },
    AChar {
        c: A_TCHEH,
        isolated: 0xfb7a,
        initial: 0xfb7c,
        medial: 0xfb7d,
        final_form: 0xfb7b,
    },
    AChar {
        c: A_JEH,
        isolated: 0xfb8a,
        initial: 0,
        medial: 0,
        final_form: 0xfb8b,
    },
    AChar {
        c: A_FKAF,
        isolated: 0xfb8e,
        initial: 0xfb90,
        medial: 0xfb91,
        final_form: 0xfb8f,
    },
    AChar {
        c: A_GAF,
        isolated: 0xfb92,
        initial: 0xfb94,
        medial: 0xfb95,
        final_form: 0xfb93,
    },
    AChar {
        c: A_FYEH,
        isolated: 0xfbfc,
        initial: 0xfbfe,
        medial: 0xfbff,
        final_form: 0xfbfd,
    },
];

#[inline]
fn find_achar(c: c_int) -> Option<&'static AChar> {
    let target = c as u32;
    ACHARS
        .binary_search_by_key(&target, |a| a.c)
        .ok()
        .map(|idx| &ACHARS[idx])
}

#[inline]
const fn chg_c_laa2i(hid_c: c_int) -> c_int {
    match hid_c as u32 {
        A_ALEF_MADDA => A_S_LAM_ALEF_MADDA_ABOVE,
        A_ALEF_HAMZA_ABOVE => A_S_LAM_ALEF_HAMZA_ABOVE,
        A_ALEF_HAMZA_BELOW => A_S_LAM_ALEF_HAMZA_BELOW,
        A_ALEF => A_S_LAM_ALEF,
        _ => 0,
    }
}

#[inline]
const fn chg_c_laa2f(hid_c: c_int) -> c_int {
    match hid_c as u32 {
        A_ALEF_MADDA => A_F_LAM_ALEF_MADDA_ABOVE,
        A_ALEF_HAMZA_ABOVE => A_F_LAM_ALEF_HAMZA_ABOVE,
        A_ALEF_HAMZA_BELOW => A_F_LAM_ALEF_HAMZA_BELOW,
        A_ALEF => A_F_LAM_ALEF,
        _ => 0,
    }
}

#[inline]
fn can_join(c1: c_int, c2: c_int) -> bool {
    let (Some(a1), Some(a2)) = (find_achar(c1), find_achar(c2)) else {
        return false;
    };
    (a1.initial != 0 || a1.medial != 0) && (a2.final_form != 0 || a2.medial != 0)
}

/// Check whether `two` could be an Arabic combining character (`arabic_maycombine`).
pub fn arabic_maycombine(two: c_int) -> bool {
    if arshape_enabled() {
        matches!(
            two as u32,
            A_ALEF_MADDA | A_ALEF_HAMZA_ABOVE | A_ALEF_HAMZA_BELOW | A_ALEF
        )
    } else {
        false
    }
}

/// Check whether `(one, two)` are Arabic combining characters (`arabic_combine`).
pub fn arabic_combine(one: c_int, two: c_int) -> bool {
    if one as u32 == A_LAM {
        arabic_maycombine(two)
    } else {
        false
    }
}

#[inline]
fn a_is_iso(c: c_int) -> bool {
    find_achar(c).is_some()
}

#[inline]
fn a_is_ok(c: c_int) -> bool {
    a_is_iso(c) || (c as u32) == A_BYTE_ORDER_MARK
}

#[inline]
fn a_is_valid(c: c_int) -> bool {
    a_is_ok(c) && (c as u32) != A_HAMZA
}

/// Perform Arabic shaping on character `c` (`arabic_shape`).
pub fn arabic_shape(
    c: c_int,
    c1p: &mut c_int,
    prev_c: c_int,
    prev_c1: c_int,
    next_c: c_int,
) -> c_int {
    if !a_is_ok(c) {
        return c;
    }

    let c1 = *c1p;
    let curr_laa = arabic_combine(c, c1);
    let prev_laa = arabic_combine(prev_c, prev_c1);

    let mut curr_c = if curr_laa {
        let shaped = if a_is_valid(prev_c) && can_join(prev_c, A_LAM as c_int) && !prev_laa {
            chg_c_laa2f(c1)
        } else {
            chg_c_laa2i(c1)
        };
        *c1p = 0;
        shaped
    } else {
        let Some(curr_a) = find_achar(c) else {
            return c;
        };
        let backward_combine = !prev_laa && can_join(prev_c, c);
        let forward_combine = can_join(c, next_c);

        let shaped = match (backward_combine, forward_combine) {
            (true, true) => curr_a.medial,
            (true, false) => curr_a.final_form,
            (false, true) => curr_a.initial,
            (false, false) => curr_a.isolated,
        };
        shaped as c_int
    };

    if curr_c == 0 {
        curr_c = c;
    }

    curr_c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn achars_table_is_strictly_sorted() {
        for pair in ACHARS.windows(2) {
            assert!(pair[0].c < pair[1].c);
        }
    }

    #[test]
    fn combine_and_shape_lam_alef_and_contextual_forms() {
        opt_vars().p_arshape = 1;
        opt_vars().p_tbidi = 0;

        assert!(arabic_maycombine(A_ALEF as c_int));
        assert!(arabic_combine(A_LAM as c_int, A_ALEF as c_int));
        assert!(!arabic_combine(A_BEH as c_int, A_ALEF as c_int));

        // LAM + ALEF isolated -> 0xfefb, clearing *c1p
        let mut c1: c_int = A_ALEF as c_int;
        let shaped = arabic_shape(A_LAM as c_int, &mut c1, 0, 0, 0);
        assert_eq!(shaped, A_S_LAM_ALEF);
        assert_eq!(c1, 0);

        // BEH followed by LAM+ALEF: LAM+ALEF becomes final form 0xfefc
        let mut c1_final: c_int = A_ALEF as c_int;
        let shaped_final = arabic_shape(A_LAM as c_int, &mut c1_final, A_BEH as c_int, 0, 0);
        assert_eq!(shaped_final, A_F_LAM_ALEF);
        assert_eq!(c1_final, 0);

        // BEH contextual forms: isolated (0xfe8f), initial (0xfe91), medial (0xfe92), final (0xfe90)
        let mut zero: c_int = 0;
        assert_eq!(arabic_shape(A_BEH as c_int, &mut zero, 0, 0, 0), 0xfe8f);
        assert_eq!(
            arabic_shape(A_BEH as c_int, &mut zero, 0, 0, A_TEH as c_int),
            0xfe91
        );
        assert_eq!(
            arabic_shape(A_BEH as c_int, &mut zero, A_TEH as c_int, 0, A_TEH as c_int),
            0xfe92
        );
        assert_eq!(
            arabic_shape(A_BEH as c_int, &mut zero, A_TEH as c_int, 0, 0),
            0xfe90
        );
    }
}
