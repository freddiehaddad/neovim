//! Vimscript compound assignment operator execution (`+=`, `-=`, `*=`, `/=`, `%=`, `.=`)
//! (`src/nvim/eval/executor.c`).

use crate::errors::*;
use crate::eval::typval::{
    TypvalVval, VarType, tv_clear, tv_get_number, tv_get_string, tv_get_string_buf, tv_list_extend,
    typval_T,
};
use crate::eval::{grow_string_tv, num_divide, num_modulus};
use crate::math::{FAIL, OK};
use crate::message::semsg;
use crate::regexp::NUMBUFLEN;
use crate::strings::concat_str;
use std::ffi::{c_char, c_int};

unsafe fn tv_op_blob(tv1: &mut typval_T, tv2: &typval_T, op: u8) -> c_int {
    if op != b'+' || tv2.v_type != VarType::VAR_BLOB {
        return FAIL;
    }

    let b2 = tv2.vval.v_blob();
    if b2.is_null() {
        return OK;
    }

    let b1 = tv1.vval.v_blob();
    if b1.is_null() {
        tv1.vval = crate::eval::typval::TypvalVval::Blob(b2);
        unsafe {
            (*b2).bv_refcount += 1;
        }
        return OK;
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

    OK
}

unsafe fn tv_op_list(tv1: &mut typval_T, tv2: &typval_T, op: u8) -> c_int {
    if op != b'+' || tv2.v_type != VarType::VAR_LIST {
        return FAIL;
    }

    let l2 = tv2.vval.v_list();
    if l2.is_null() {
        return OK;
    }

    let l1 = tv1.vval.v_list();
    if l1.is_null() {
        tv1.vval = crate::eval::typval::TypvalVval::List(l2);
        unsafe {
            (*l2).lv_refcount += 1;
        }
    } else {
        unsafe {
            tv_list_extend(l1, l2, std::ptr::null_mut());
        }
    }

    OK
}

unsafe fn tv_op_number(tv1: &mut typval_T, tv2: &typval_T, op: u8) -> c_int {
    let mut n = unsafe { tv_get_number(tv1) };

    if tv2.v_type == VarType::VAR_FLOAT {
        let mut f = n as f64;
        if op == b'%' {
            return FAIL;
        }
        let rhs = tv2.vval.v_float();
        match op {
            b'+' => f += rhs,
            b'-' => f -= rhs,
            b'*' => f *= rhs,
            b'/' => f /= rhs,
            _ => {}
        }
        unsafe {
            tv_clear(tv1);
        }
        tv1.v_type = VarType::VAR_FLOAT;
        tv1.vval = TypvalVval::Float(f);
    } else {
        let rhs = unsafe { tv_get_number(tv2) };

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
        unsafe {
            tv_clear(tv1);
        }
        tv1.v_type = VarType::VAR_NUMBER;
        tv1.vval = TypvalVval::Number(n);
    }

    OK
}

unsafe fn tv_op_string(tv1: &mut typval_T, tv2: &typval_T) -> c_int {
    if tv2.v_type == VarType::VAR_FLOAT {
        return FAIL;
    }

    unsafe {
        let mut numbuf = [0 as c_char; NUMBUFLEN];
        let s2 = tv_get_string_buf(tv2, numbuf.as_mut_ptr());
        if grow_string_tv(tv1, s2) == OK {
            return OK;
        }

        let tvs = tv_get_string(tv1);
        let s = concat_str(tvs, s2);
        crate::eval::typval::tv_set_string(tv1, s.as_ptr());
    }

    OK
}

unsafe fn tv_op_nr_or_string(tv1: &mut typval_T, tv2: &typval_T, op: u8) -> c_int {
    if tv2.v_type == VarType::VAR_LIST {
        return FAIL;
    }
    if matches!(op, b'+' | b'-' | b'*' | b'/' | b'%') {
        unsafe { tv_op_number(tv1, tv2, op) }
    } else {
        unsafe { tv_op_string(tv1, tv2) }
    }
}

unsafe fn tv_op_float(tv1: &mut typval_T, tv2: &typval_T, op: u8) -> c_int {
    if op == b'%'
        || op == b'.'
        || !matches!(
            tv2.v_type,
            VarType::VAR_FLOAT | VarType::VAR_NUMBER | VarType::VAR_STRING
        )
    {
        return FAIL;
    }

    let f = if tv2.v_type == VarType::VAR_FLOAT {
        tv2.vval.v_float()
    } else {
        (unsafe { tv_get_number(tv2) }) as f64
    };

    match op {
        b'+' => *tv1.vval.v_float_mut() += f,
        b'-' => *tv1.vval.v_float_mut() -= f,
        b'*' => *tv1.vval.v_float_mut() *= f,
        b'/' => *tv1.vval.v_float_mut() /= f,
        _ => {}
    }

    OK
}

/// Execute compound assignment `tv1 <op>= tv2` (`eexe_mod_op`).
pub unsafe fn eexe_mod_op(tv1: *mut typval_T, tv2: *const typval_T, op: *const c_char) -> c_int {
    let tv1 = unsafe { &mut *tv1 };
    let tv2 = unsafe { &*tv2 };
    let op_byte = unsafe { *op as u8 };

    if tv2.v_type == VarType::VAR_FUNC
        || tv2.v_type == VarType::VAR_DICT
        || (matches!(tv2.v_type, VarType::VAR_BOOL | VarType::VAR_SPECIAL) && op_byte == b'.')
    {
        unsafe {
            semsg!(e_letwrong.as_ptr(), op);
        }
        return FAIL;
    }

    let retval = match tv1.v_type {
        VarType::VAR_DICT
        | VarType::VAR_FUNC
        | VarType::VAR_PARTIAL
        | VarType::VAR_BOOL
        | VarType::VAR_SPECIAL => FAIL,
        VarType::VAR_BLOB => unsafe { tv_op_blob(tv1, tv2, op_byte) },
        VarType::VAR_LIST => unsafe { tv_op_list(tv1, tv2, op_byte) },
        VarType::VAR_NUMBER | VarType::VAR_STRING => unsafe {
            tv_op_nr_or_string(tv1, tv2, op_byte)
        },
        VarType::VAR_FLOAT => unsafe { tv_op_float(tv1, tv2, op_byte) },
        VarType::VAR_UNKNOWN => std::process::abort(),
    };

    if retval != OK {
        unsafe {
            semsg!(e_letwrong.as_ptr(), op);
        }
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
        unsafe {
            assert_eq!(eexe_mod_op(&mut tv1, &tv2, c"+".as_ptr()), OK);
            assert_eq!(tv1.vval.v_number(), 15);
            assert_eq!(eexe_mod_op(&mut tv1, &tv2, c"*".as_ptr()), OK);
            assert_eq!(tv1.vval.v_number(), 75);
        }
    }
}
