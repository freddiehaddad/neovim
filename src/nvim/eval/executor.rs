//! Vimscript compound assignment operator execution (`+=`, `-=`, `*=`, `/=`, `%=`, `.=`)
//! (`src/nvim/eval/executor.c`).

#![allow(dead_code, unused_imports, clippy::missing_safety_doc)]

use std::ffi::{c_char, c_int, c_void};

use crate::clipboard::{TypvalVval, VarType, list_T, typval_T};
use crate::garray::{ga_grow, garray_T};
use crate::math::{FAIL, OK};

const NUMBUFLEN: usize = 65;

#[repr(C)]
#[allow(non_camel_case_types)]
pub struct blob_T {
    pub bv_ga: garray_T,
    pub bv_refcount: c_int,
    pub bv_lock: c_int,
}

#[cfg(not(test))]
unsafe extern "C" {
    static e_letwrong: [c_char; 0];

    fn tv_list_extend(l1: *mut list_T, l2: *mut list_T, bef: *mut c_void) -> c_int;
    fn tv_get_number(varp: *const typval_T) -> i64;
    fn tv_clear(tv: *mut typval_T);
    fn num_divide(n1: i64, n2: i64) -> i64;
    fn num_modulus(n1: i64, n2: i64) -> i64;
    fn tv_get_string_buf(varp: *const typval_T, buf: *mut c_char) -> *const c_char;
    fn grow_string_tv(tv: *mut typval_T, s: *const c_char) -> c_int;
    fn tv_get_string(varp: *const typval_T) -> *const c_char;
    fn concat_str(str1: *const c_char, str2: *const c_char) -> *mut c_char;
    fn semsg(fmt: *const c_char, ...) -> bool;
}

unsafe fn tv_op_blob(tv1: &mut typval_T, tv2: &typval_T, op: u8) -> c_int {
    if op != b'+' || tv2.v_type != VarType::VAR_BLOB {
        return FAIL;
    }

    let b2 = unsafe { tv2.vval.v_ptr.cast::<blob_T>() };
    if b2.is_null() {
        return OK;
    }

    let b1 = unsafe { tv1.vval.v_ptr.cast::<blob_T>() };
    if b1.is_null() {
        tv1.vval.v_ptr = b2.cast::<c_void>();
        unsafe {
            (*b2).bv_refcount += 1;
        }
        return OK;
    }

    unsafe {
        let len = (*b2).bv_ga.ga_len;
        if len > 0 {
            ga_grow(&mut (*b1).bv_ga, len);
            std::ptr::copy(
                (*b2).bv_ga.ga_data.cast::<u8>(),
                (*b1)
                    .bv_ga
                    .ga_data
                    .cast::<u8>()
                    .add((*b1).bv_ga.ga_len as usize),
                len as usize,
            );
            (*b1).bv_ga.ga_len += len;
        }
    }

    OK
}

unsafe fn tv_op_list(tv1: &mut typval_T, tv2: &typval_T, op: u8) -> c_int {
    if op != b'+' || tv2.v_type != VarType::VAR_LIST {
        return FAIL;
    }

    let l2 = unsafe { tv2.vval.v_list };
    if l2.is_null() {
        return OK;
    }

    let l1 = unsafe { tv1.vval.v_list };
    if l1.is_null() {
        tv1.vval.v_list = l2;
        unsafe {
            (*l2).lv_refcount += 1;
        }
    } else {
        #[cfg(not(test))]
        unsafe {
            tv_list_extend(l1, l2, std::ptr::null_mut());
        }
    }

    OK
}

unsafe fn tv_op_number(tv1: &mut typval_T, tv2: &typval_T, op: u8) -> c_int {
    #[cfg(test)]
    let mut n = if tv1.v_type == VarType::VAR_NUMBER {
        unsafe { tv1.vval.v_number }
    } else {
        0
    };
    #[cfg(not(test))]
    let mut n = unsafe { tv_get_number(tv1) };

    if tv2.v_type == VarType::VAR_FLOAT {
        let mut f = n as f64;
        if op == b'%' {
            return FAIL;
        }
        let rhs = unsafe { tv2.vval.v_float };
        match op {
            b'+' => f += rhs,
            b'-' => f -= rhs,
            b'*' => f *= rhs,
            b'/' => f /= rhs,
            _ => {}
        }
        #[cfg(not(test))]
        unsafe {
            tv_clear(tv1);
        }
        tv1.v_type = VarType::VAR_FLOAT;
        tv1.vval = TypvalVval { v_float: f };
    } else {
        #[cfg(test)]
        let rhs = unsafe { tv2.vval.v_number };
        #[cfg(not(test))]
        let rhs = unsafe { tv_get_number(tv2) };

        match op {
            b'+' => n = n.wrapping_add(rhs),
            b'-' => n = n.wrapping_sub(rhs),
            b'*' => n = n.wrapping_mul(rhs),
            b'/' => {
                #[cfg(not(test))]
                {
                    n = unsafe { num_divide(n, rhs) };
                }
                #[cfg(test)]
                {
                    n /= rhs;
                }
            }
            b'%' => {
                #[cfg(not(test))]
                {
                    n = unsafe { num_modulus(n, rhs) };
                }
                #[cfg(test)]
                {
                    n %= rhs;
                }
            }
            _ => {}
        }
        #[cfg(not(test))]
        unsafe {
            tv_clear(tv1);
        }
        tv1.v_type = VarType::VAR_NUMBER;
        tv1.vval = TypvalVval { v_number: n };
    }

    OK
}

unsafe fn tv_op_string(tv1: &mut typval_T, tv2: &typval_T) -> c_int {
    if tv2.v_type == VarType::VAR_FLOAT {
        return FAIL;
    }

    #[cfg(not(test))]
    unsafe {
        let mut numbuf = [0 as c_char; NUMBUFLEN];
        let s2 = tv_get_string_buf(tv2, numbuf.as_mut_ptr());
        if grow_string_tv(tv1, s2) == OK {
            return OK;
        }

        let tvs = tv_get_string(tv1);
        let s = concat_str(tvs, s2);
        tv_clear(tv1);
        tv1.v_type = VarType::VAR_STRING;
        tv1.vval = TypvalVval { v_string: s };
    }
    #[cfg(test)]
    let _ = (tv1, NUMBUFLEN);

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
        unsafe { tv2.vval.v_float }
    } else {
        #[cfg(not(test))]
        {
            (unsafe { tv_get_number(tv2) }) as f64
        }
        #[cfg(test)]
        {
            (unsafe { tv2.vval.v_number }) as f64
        }
    };

    unsafe {
        match op {
            b'+' => tv1.vval.v_float += f,
            b'-' => tv1.vval.v_float -= f,
            b'*' => tv1.vval.v_float *= f,
            b'/' => tv1.vval.v_float /= f,
            _ => {}
        }
    }

    OK
}

/// Execute compound assignment `tv1 <op>= tv2` (`eexe_mod_op`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eexe_mod_op(
    tv1: *mut typval_T,
    tv2: *const typval_T,
    op: *const c_char,
) -> c_int {
    let tv1 = unsafe { &mut *tv1 };
    let tv2 = unsafe { &*tv2 };
    let op_byte = unsafe { *op as u8 };

    if tv2.v_type == VarType::VAR_FUNC
        || tv2.v_type == VarType::VAR_DICT
        || (matches!(tv2.v_type, VarType::VAR_BOOL | VarType::VAR_SPECIAL) && op_byte == b'.')
    {
        #[cfg(not(test))]
        unsafe {
            semsg(e_letwrong.as_ptr(), op);
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
        #[cfg(not(test))]
        unsafe {
            semsg(e_letwrong.as_ptr(), op);
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
            vval: TypvalVval { v_number: 10 },
        };
        let tv2 = typval_T {
            v_type: VarType::VAR_NUMBER,
            v_lock: 0,
            vval: TypvalVval { v_number: 5 },
        };
        unsafe {
            assert_eq!(eexe_mod_op(&mut tv1, &tv2, c"+".as_ptr()), OK);
            assert_eq!(tv1.vval.v_number, 15);
            assert_eq!(eexe_mod_op(&mut tv1, &tv2, c"*".as_ptr()), OK);
            assert_eq!(tv1.vval.v_number, 75);
        }
    }
}
