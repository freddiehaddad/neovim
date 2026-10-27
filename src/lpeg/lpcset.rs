//! Character set operations (`src/lpeg/lpcset.c`).

use std::ffi::c_int;

use crate::lpeg::{
    BITSPERCHAR, CHARSETSIZE, Charset, IAny, IChar, IFail, ISet, Opcode, TAny, TChar, TFalse, TSet,
    TTree, byte, charsetinfo, clearset, fillset, setchar, treebuffer,
};

fn onlybit(mut c: c_int, mut b: c_int) -> c_int {
    if (b & 0xF0) != 0 {
        c += 4;
        b >>= 4;
    }
    if (b & 0x0C) != 0 {
        c += 2;
        b >>= 2;
    }
    if (b & 0x02) != 0 {
        c += 1;
    }
    c
}

pub fn charsettype(cs: *const byte, info: *mut charsetinfo) -> Opcode {
    unsafe {
        let mut low1 = 0usize;
        while low1 < CHARSETSIZE && *cs.add(low1) == 0 {
            low1 += 1;
        }
        if low1 == CHARSETSIZE {
            return IFail;
        }
        let mut high1 = CHARSETSIZE - 1;
        while *cs.add(high1) == 0 {
            high1 -= 1;
        }
        if low1 == high1 {
            let b = c_int::from(*cs.add(low1));
            if (b & (b - 1)) == 0 {
                (*info).offset = onlybit((low1 * BITSPERCHAR) as c_int, b);
                return IChar;
            }
        }
        let mut low0 = 0usize;
        while low0 < CHARSETSIZE && *cs.add(low0) == 0xFF {
            low0 += 1;
        }
        if low0 == CHARSETSIZE {
            return IAny;
        }
        let mut high0 = CHARSETSIZE - 1;
        while *cs.add(high0) == 0xFF {
            high0 -= 1;
        }
        if high1 - low1 <= high0 - low0 {
            (*info).offset = low1 as c_int;
            (*info).size = (high1 - low1 + 1) as c_int;
            (*info).deflt = 0;
        } else {
            (*info).offset = low0 as c_int;
            (*info).size = (high0 - low0 + 1) as c_int;
            (*info).deflt = 0xFF;
        }
        (*info).cs = cs.add((*info).offset as usize);
        ISet
    }
}

pub fn getbytefromcharset(info: *const charsetinfo, index: c_int) -> byte {
    unsafe {
        if index < (*info).size {
            *(*info).cs.add(index as usize)
        } else {
            (*info).deflt as byte
        }
    }
}

pub fn tocharset(tree: *mut TTree, cs: *mut Charset) -> c_int {
    unsafe {
        let cs_ptr = (*cs).cs.as_mut_ptr();
        match (*tree).tag {
            TChar => {
                debug_assert!(0 <= (*tree).u.n && (*tree).u.n <= c_int::from(u8::MAX));
                clearset(cs_ptr);
                setchar(cs_ptr, (*tree).u.n as usize);
                1
            }
            TAny => {
                fillset(cs_ptr, 0xFF);
                1
            }
            TFalse => {
                clearset(cs_ptr);
                1
            }
            TSet => {
                fillset(cs_ptr, (*tree).u.set.deflt);
                let tb = treebuffer(tree);
                for i in 0..((*tree).u.set.size as usize) {
                    *cs_ptr.add(((*tree).u.set.offset as usize) + i) = *tb.add(i);
                }
                1
            }
            _ => 0,
        }
    }
}

pub fn tree2cset(tree: *mut TTree, info: *mut charsetinfo) {
    unsafe {
        debug_assert_eq!((*tree).tag, TSet);
        (*info).offset = c_int::from((*tree).u.set.offset);
        (*info).size = c_int::from((*tree).u.set.size);
        (*info).deflt = c_int::from((*tree).u.set.deflt);
        (*info).cs = treebuffer(tree);
    }
}
