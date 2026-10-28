//! Vimscript compound assignment operator execution (`+=`, `-=`, `*=`, `/=`, `%=`, `.=`)
//! (`src/nvim/eval/executor.c`).

use crate::errors::*;
use crate::eval::typval::{
    TypVal, TypvalVval, VarType, tv_clear, tv_get_number, tv_get_string, tv_get_string_buf,
    tv_list_extend,
};
use crate::eval::{grow_string_tv, num_divide, num_modulus};
use crate::message::semsg;
use crate::regexp::NUMBUFLEN;
use crate::strings::concat_str;
use std::ffi::c_char;

fn tv_op_blob(tv1: &mut TypVal, tv2: &TypVal, op: u8) -> bool {
    if op != b'+' || tv2.v_type != VarType::VarBlob {
        return false;
    }

    let b2 = tv2.vval.v_blob();
    if b2.is_null() {
        return true;
    }
    let b2 = unsafe { &mut *b2 };

    let b1 = tv1.vval.v_blob();
    if b1.is_null() {
        tv1.vval = crate::eval::typval::TypvalVval::Blob(b2);
        {
            b2.bv_refcount += 1;
        }
        return true;
    }

    unsafe {
        if !b2.bv_ga.is_empty() {
            let b1_ref = &mut *b1;
            if std::ptr::eq(b1_ref, b2) {
                let copy = b1_ref.bv_ga.clone();
                b1_ref.bv_ga.extend_from_slice(&copy);
            } else {
                b1_ref.bv_ga.extend_from_slice(&b2.bv_ga);
            }
        }
    }

    true
}

fn tv_op_list(tv1: &mut TypVal, tv2: &TypVal, op: u8) -> bool {
    if op != b'+' || tv2.v_type != VarType::VarList {
        return false;
    }

    let l2 = tv2.vval.v_list();
    if l2.is_null() {
        return true;
    }

    let l1 = tv1.vval.v_list();
    if l1.is_null() {
        tv1.vval = crate::eval::typval::TypvalVval::List(l2);
        unsafe {
            (*l2).lv_refcount += 1;
        }
    } else {
        tv_list_extend(
            unsafe { &mut *l1 },
            unsafe { l2.as_ref() },
            std::ptr::null_mut(),
        );
    }

    true
}

fn tv_op_number(tv1: &mut TypVal, tv2: &TypVal, op: u8) -> bool {
    let mut n = tv_get_number(&*tv1);

    if tv2.v_type == VarType::VarFloat {
        let mut f = n as f64;
        if op == b'%' {
            return false;
        }
        let rhs = tv2.vval.v_float();
        match op {
            b'+' => f += rhs,
            b'-' => f -= rhs,
            b'*' => f *= rhs,
            b'/' => f /= rhs,
            _ => {}
        }
        tv_clear(tv1);
        tv1.v_type = VarType::VarFloat;
        tv1.vval = TypvalVval::Float(f);
    } else {
        let rhs = tv_get_number(tv2);

        match op {
            b'+' => n = n.wrapping_add(rhs),
            b'-' => n = n.wrapping_sub(rhs),
            b'*' => n = n.wrapping_mul(rhs),
            b'/' => {
                n = num_divide(n, rhs);
            }
            b'%' => {
                n = num_modulus(n, rhs);
            }
            _ => {}
        }
        tv_clear(tv1);
        tv1.v_type = VarType::VarNumber;
        tv1.vval = TypvalVval::Number(n);
    }

    true
}

fn tv_op_string(tv1: &mut TypVal, tv2: &TypVal) -> bool {
    if tv2.v_type == VarType::VarFloat {
        return false;
    }

    {
        let mut numbuf = [0 as c_char; NUMBUFLEN];
        let s2 = tv_get_string_buf(tv2, numbuf.as_mut_ptr());
        if grow_string_tv(&mut *tv1, s2) {
            return true;
        }

        let tvs = tv_get_string(&*tv1);
        let s = concat_str(tvs, s2);
        crate::eval::typval::tv_set_string(tv1, s.as_ptr());
    }

    true
}

fn tv_op_nr_or_string(tv1: &mut TypVal, tv2: &TypVal, op: u8) -> bool {
    if tv2.v_type == VarType::VarList {
        return false;
    }
    if matches!(op, b'+' | b'-' | b'*' | b'/' | b'%') {
        tv_op_number(tv1, tv2, op)
    } else {
        tv_op_string(tv1, tv2)
    }
}

fn tv_op_float(tv1: &mut TypVal, tv2: &TypVal, op: u8) -> bool {
    if op == b'%'
        || op == b'.'
        || !matches!(
            tv2.v_type,
            VarType::VarFloat | VarType::VarNumber | VarType::VarString
        )
    {
        return false;
    }

    let f = if tv2.v_type == VarType::VarFloat {
        tv2.vval.v_float()
    } else {
        (tv_get_number(tv2)) as f64
    };

    match op {
        b'+' => *tv1.vval.v_float_mut() += f,
        b'-' => *tv1.vval.v_float_mut() -= f,
        b'*' => *tv1.vval.v_float_mut() *= f,
        b'/' => *tv1.vval.v_float_mut() /= f,
        _ => {}
    }

    true
}

/// Execute compound assignment `tv1 <op>= tv2` (`eexe_mod_op`).
pub fn eexe_mod_op(tv1: &mut TypVal, tv2: &TypVal, op: impl crate::strings::IntoCCharPtr) -> bool {
    let op = op.into_c_char_ptr();
    let tv1 = { tv1 };
    let tv2 = { tv2 };
    let op_byte = { crate::memory::cstr_first_byte(op) };

    if tv2.v_type == VarType::VarFunc
        || tv2.v_type == VarType::VarDict
        || (matches!(tv2.v_type, VarType::VarBool | VarType::VarSpecial) && op_byte == b'.')
    {
        semsg!(E_LETWRONG, op);
        return false;
    }

    let retval = match tv1.v_type {
        VarType::VarDict
        | VarType::VarFunc
        | VarType::VarPartial
        | VarType::VarBool
        | VarType::VarSpecial => false,
        VarType::VarBlob => tv_op_blob(tv1, tv2, op_byte),
        VarType::VarList => tv_op_list(tv1, tv2, op_byte),
        VarType::VarNumber | VarType::VarString => tv_op_nr_or_string(tv1, tv2, op_byte),
        VarType::VarFloat => tv_op_float(tv1, tv2, op_byte),
        VarType::VarUnknown => std::process::abort(),
    };

    if !retval {
        semsg!(E_LETWRONG, op);
    }

    retval
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_and_float_mod_ops() {
        let mut tv1 = TypVal {
            v_type: VarType::VarNumber,
            v_lock: 0,
            vval: TypvalVval::Number(10),
        };
        let tv2 = TypVal {
            v_type: VarType::VarNumber,
            v_lock: 0,
            vval: TypvalVval::Number(5),
        };
        assert!(eexe_mod_op(&mut tv1, &tv2, c"+"));
        assert_eq!(tv1.vval.v_number(), 15);
        assert!(eexe_mod_op(&mut tv1, &tv2, c"*"));
        assert_eq!(tv1.vval.v_number(), 75);
    }
}
