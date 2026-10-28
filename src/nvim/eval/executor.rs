//! Vimscript compound assignment operator execution (`+=`, `-=`, `*=`, `/=`, `%=`, `.=`)
//! (`src/nvim/eval/executor.c`).

use crate::errors::*;
use crate::eval::typval::{
    TypvalVval, VarType, tv_clear, tv_get_number, tv_get_string, tv_get_string_buf, tv_list_extend,
    typval_T,
};
use crate::eval::{grow_string_tv, num_divide, num_modulus};
use crate::message::semsg;
use crate::regexp::NUMBUFLEN;
use crate::strings::concat_str;
use std::ffi::c_char;

fn tv_op_blob(tv1: &mut typval_T, tv2: &typval_T, op: u8) -> bool {
    if op != b'+' || tv2.v_type != VarType::VAR_BLOB {
        return false;
    }

    let b2 = tv2.vval.v_blob();
    if b2.is_null() {
        return true;
    }

    let b1 = tv1.vval.v_blob();
    if b1.is_null() {
        tv1.vval = crate::eval::typval::TypvalVval::Blob(b2);
        unsafe {
            (*b2).bv_refcount += 1;
        }
        return true;
    }

    unsafe {
        if !(*b2).bv_ga.is_empty() {
            let b1_ref = &mut *b1;
            if std::ptr::eq(b1_ref, b2) {
                let copy = b1_ref.bv_ga.clone();
                b1_ref.bv_ga.extend_from_slice(&copy);
            } else {
                b1_ref.bv_ga.extend_from_slice(&(*b2).bv_ga);
            }
        }
    }

    true
}

fn tv_op_list(tv1: &mut typval_T, tv2: &typval_T, op: u8) -> bool {
    if op != b'+' || tv2.v_type != VarType::VAR_LIST {
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
        tv_list_extend(l1, l2, std::ptr::null_mut());
    }

    true
}

fn tv_op_number(tv1: &mut typval_T, tv2: &typval_T, op: u8) -> bool {
    let mut n = tv_get_number(&*tv1);

    if tv2.v_type == VarType::VAR_FLOAT {
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
        tv1.v_type = VarType::VAR_FLOAT;
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
        tv1.v_type = VarType::VAR_NUMBER;
        tv1.vval = TypvalVval::Number(n);
    }

    true
}

fn tv_op_string(tv1: &mut typval_T, tv2: &typval_T) -> bool {
    if tv2.v_type == VarType::VAR_FLOAT {
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

fn tv_op_nr_or_string(tv1: &mut typval_T, tv2: &typval_T, op: u8) -> bool {
    if tv2.v_type == VarType::VAR_LIST {
        return false;
    }
    if matches!(op, b'+' | b'-' | b'*' | b'/' | b'%') {
        tv_op_number(tv1, tv2, op)
    } else {
        tv_op_string(tv1, tv2)
    }
}

fn tv_op_float(tv1: &mut typval_T, tv2: &typval_T, op: u8) -> bool {
    if op == b'%'
        || op == b'.'
        || !matches!(
            tv2.v_type,
            VarType::VAR_FLOAT | VarType::VAR_NUMBER | VarType::VAR_STRING
        )
    {
        return false;
    }

    let f = if tv2.v_type == VarType::VAR_FLOAT {
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
pub fn eexe_mod_op(tv1: &mut typval_T,tv2: &typval_T, op: impl crate::strings::IntoCCharPtr) -> bool {
    let op = op.into_c_char_ptr();
    let tv1 =  { tv1 };
    let tv2 =  { tv2 };
    let op_byte = unsafe { *op as u8 };

    if tv2.v_type == VarType::VAR_FUNC
        || tv2.v_type == VarType::VAR_DICT
        || (matches!(tv2.v_type, VarType::VAR_BOOL | VarType::VAR_SPECIAL) && op_byte == b'.')
    {
        semsg!(e_letwrong, op);
        return false;
    }

    let retval = match tv1.v_type {
        VarType::VAR_DICT
        | VarType::VAR_FUNC
        | VarType::VAR_PARTIAL
        | VarType::VAR_BOOL
        | VarType::VAR_SPECIAL => false,
        VarType::VAR_BLOB => tv_op_blob(tv1, tv2, op_byte),
        VarType::VAR_LIST => tv_op_list(tv1, tv2, op_byte),
        VarType::VAR_NUMBER | VarType::VAR_STRING => tv_op_nr_or_string(tv1, tv2, op_byte),
        VarType::VAR_FLOAT => tv_op_float(tv1, tv2, op_byte),
        VarType::VAR_UNKNOWN => std::process::abort(),
    };

    if !retval {
        semsg!(e_letwrong, op);
    }

    retval
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_and_float_mod_ops() {
        let mut tv1 = typval_T {
            v_type: VarType::VAR_NUMBER,
            v_lock: 0,
            vval: TypvalVval::Number(10),
        };
        let tv2 = typval_T {
            v_type: VarType::VAR_NUMBER,
            v_lock: 0,
            vval: TypvalVval::Number(5),
        };
        assert!(eexe_mod_op(&mut tv1, &tv2, c"+"));
            assert_eq!(tv1.vval.v_number(), 15);
            assert!(eexe_mod_op(&mut tv1, &tv2, c"*"));
            assert_eq!(tv1.vval.v_number(), 75);
    }
}
