//! Character set operations (`src/lpeg/lpcset.c`).

use crate::lpeg::{
    BITSPERCHAR, CHARSETSIZE, Charset, CharsetInfo, IANY, ICHAR, IFAIL, ISET, LpegByte, Opcode,
    TANY, TCHAR, TFALSE, TSET, TTree, clearset, fillset, setchar, treebuffer,
};

fn onlybit(mut c: i32, mut b: i32) -> i32 {
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

pub fn charsettype(cs: &[LpegByte; CHARSETSIZE], info: &mut CharsetInfo) -> Opcode {
    let mut low1 = 0usize;
    while low1 < CHARSETSIZE && cs[low1] == 0 {
        low1 += 1;
    }
    if low1 == CHARSETSIZE {
        return IFAIL;
    }
    let mut high1 = CHARSETSIZE - 1;
    while cs[high1] == 0 {
        high1 -= 1;
    }
    if low1 == high1 {
        let b = i32::from(cs[low1]);
        if (b & (b - 1)) == 0 {
            info.offset = onlybit((low1 * BITSPERCHAR) as i32, b);
            return ICHAR;
        }
    }
    let mut low0 = 0usize;
    while low0 < CHARSETSIZE && cs[low0] == 0xFF {
        low0 += 1;
    }
    if low0 == CHARSETSIZE {
        return IANY;
    }
    let mut high0 = CHARSETSIZE - 1;
    while cs[high0] == 0xFF {
        high0 -= 1;
    }
    if high1 - low1 <= high0 - low0 {
        info.offset = low1 as i32;
        info.size = (high1 - low1 + 1) as i32;
        info.deflt = 0;
    } else {
        info.offset = low0 as i32;
        info.size = (high0 - low0 + 1) as i32;
        info.deflt = 0xFF;
    }
    info.cs = cs.as_ptr().wrapping_add(info.offset as usize);
    ISET
}

pub fn getbytefromcharset(info: &CharsetInfo, index: i32) -> LpegByte {
    if index < info.size {
        unsafe { *info.cs.wrapping_add(index as usize) }
    } else {
        info.deflt as LpegByte
    }
}

pub fn tocharset(tree: &mut TTree, cs: &mut Charset) -> i32 {
    unsafe {
        match tree.tag {
            TCHAR => {
                debug_assert!(0 <= tree.u.n && tree.u.n <= i32::from(u8::MAX));
                clearset(&mut cs.cs);
                setchar(&mut cs.cs, tree.u.n as usize);
                1
            }
            TANY => {
                fillset(&mut cs.cs, 0xFF);
                1
            }
            TFALSE => {
                clearset(&mut cs.cs);
                1
            }
            TSET => {
                fillset(&mut cs.cs, tree.u.set.deflt);
                let tb = treebuffer(tree);
                for i in 0..(tree.u.set.size as usize) {
                    cs.cs[(tree.u.set.offset as usize) + i] = *tb.wrapping_add(i);
                }
                1
            }
            _ => 0,
        }
    }
}

pub fn tree2cset(tree: &mut TTree, info: &mut CharsetInfo) {
    unsafe {
        debug_assert_eq!(tree.tag, TSET);
        info.offset = i32::from(tree.u.set.offset);
        info.size = i32::from(tree.u.set.size);
        info.deflt = i32::from(tree.u.set.deflt);
        info.cs = treebuffer(&raw mut *tree);
    }
}
