//! Patience diff algorithm (`src/xdiff/xpatience.c`).

use std::ffi::{c_char, c_int, c_ulong, c_void};
use std::mem::{size_of, zeroed};
use std::ptr;

use crate::memory::{xfree, xmalloc};
use crate::xdiff::xprepare::xdl_prepare_env;
use crate::xdiff::xutils::xdl_fall_back_diff;
use crate::xdiff::{XDF_DIFF_ALGORITHM_MASK, mmfile_t, xdfenv_t, xpparam_t};

unsafe extern "C" {
    fn strlen(s: *const c_char) -> usize;
    fn strncmp(s1: *const c_char, s2: *const c_char, n: usize) -> c_int;
}

const NON_UNIQUE: c_ulong = c_ulong::MAX;

#[repr(C)]
#[derive(Copy, Clone)]
struct Entry {
    hash: c_ulong,
    line1: c_ulong,
    line2: c_ulong,
    next: *mut Entry,
    previous: *mut Entry,
    anchor: u8,
}

#[repr(C)]
struct HashMap {
    nr: c_int,
    alloc: c_int,
    entries: *mut Entry,
    first: *mut Entry,
    last: *mut Entry,
    has_matches: c_ulong,
    file1: *mut mmfile_t,
    file2: *mut mmfile_t,
    env: *mut xdfenv_t,
    xpp: *const xpparam_t,
}

unsafe fn is_anchor(xpp: *const xpparam_t, line: *const c_char) -> u8 {
    unsafe {
        for i in 0..(*xpp).anchors_nr {
            let anchor = *(*xpp).anchors.add(i);
            if strncmp(line, anchor, strlen(anchor)) == 0 {
                return 1;
            }
        }
    }
    0
}

unsafe fn insert_record(xpp: *const xpparam_t, line: c_int, map: *mut HashMap, pass: c_int) {
    unsafe {
        let records = if pass == 1 {
            (*(*map).env).xdf1.recs
        } else {
            (*(*map).env).xdf2.recs
        };
        let record = *records.offset((line - 1) as isize);
        let mut index = (((*record).ha << 1) % ((*map).alloc as c_ulong)) as isize;

        while (*(*map).entries.offset(index)).line1 != 0 {
            if (*(*map).entries.offset(index)).hash != (*record).ha {
                index += 1;
                if index >= (*map).alloc as isize {
                    index = 0;
                }
                continue;
            }
            if pass == 2 {
                (*map).has_matches = 1;
            }
            if pass == 1 || (*(*map).entries.offset(index)).line2 != 0 {
                (*(*map).entries.offset(index)).line2 = NON_UNIQUE;
            } else {
                (*(*map).entries.offset(index)).line2 = line as c_ulong;
            }
            return;
        }
        if pass == 2 {
            return;
        }
        let ent = (*map).entries.offset(index);
        (*ent).line1 = line as c_ulong;
        (*ent).hash = (*record).ha;
        (*ent).anchor = is_anchor(
            xpp,
            (**(*(*map).env).xdf1.recs.offset((line - 1) as isize)).ptr,
        );
        if (*map).first.is_null() {
            (*map).first = ent;
        }
        if !(*map).last.is_null() {
            (*(*map).last).next = ent;
            (*ent).previous = (*map).last;
        }
        (*map).last = ent;
        (*map).nr += 1;
    }
}

unsafe fn fill_hashmap(
    file1: *mut mmfile_t,
    file2: *mut mmfile_t,
    xpp: *const xpparam_t,
    env: *mut xdfenv_t,
    result: *mut HashMap,
    mut line1: c_int,
    mut count1: c_int,
    mut line2: c_int,
    mut count2: c_int,
) -> c_int {
    unsafe {
        (*result).file1 = file1;
        (*result).file2 = file2;
        (*result).xpp = xpp;
        (*result).env = env;

        (*result).alloc = count1 * 2;
        (*result).entries =
            xmalloc(((*result).alloc as usize) * size_of::<Entry>()).cast::<Entry>();
        if (*result).entries.is_null() {
            return -1;
        }
        ptr::write_bytes((*result).entries, 0, (*result).alloc as usize);

        while count1 > 0 {
            count1 -= 1;
            insert_record(xpp, line1, result, 1);
            line1 += 1;
        }

        while count2 > 0 {
            count2 -= 1;
            insert_record(xpp, line2, result, 2);
            line2 += 1;
        }
    }
    0
}

unsafe fn binary_search(sequence: *mut *mut Entry, longest: c_int, entry: *mut Entry) -> c_int {
    let mut left: c_int = -1;
    let mut right: c_int = longest;

    unsafe {
        while left + 1 < right {
            let middle = left + (right - left) / 2;
            if (**sequence.offset(middle as isize)).line2 > (*entry).line2 {
                right = middle;
            } else {
                left = middle;
            }
        }
    }
    left
}

unsafe fn find_longest_common_sequence(map: *mut HashMap) -> *mut Entry {
    unsafe {
        let sequence = xmalloc(((*map).nr as usize) * size_of::<*mut Entry>()).cast::<*mut Entry>();
        if sequence.is_null() {
            return (*map).first;
        }

        let mut longest: c_int = 0;
        let mut anchor_i: c_int = -1;
        let mut entry = (*map).first;

        while !entry.is_null() {
            if (*entry).line2 != 0 && (*entry).line2 != NON_UNIQUE {
                let mut i = binary_search(sequence, longest, entry);
                (*entry).previous = if i < 0 {
                    ptr::null_mut()
                } else {
                    *sequence.offset(i as isize)
                };
                i += 1;
                if i > anchor_i {
                    *sequence.offset(i as isize) = entry;
                    if (*entry).anchor != 0 {
                        anchor_i = i;
                        longest = anchor_i + 1;
                    } else if i == longest {
                        longest += 1;
                    }
                }
            }
            entry = (*entry).next;
        }

        if longest == 0 {
            xfree(sequence.cast::<c_void>());
            return ptr::null_mut();
        }

        entry = *sequence.offset((longest - 1) as isize);
        (*entry).next = ptr::null_mut();
        while !(*entry).previous.is_null() {
            (*(*entry).previous).next = entry;
            entry = (*entry).previous;
        }
        xfree(sequence.cast::<c_void>());
        entry
    }
}

unsafe fn match_lines(map: *mut HashMap, line1: c_int, line2: c_int) -> bool {
    unsafe {
        let record1 = *(*(*map).env).xdf1.recs.offset((line1 - 1) as isize);
        let record2 = *(*(*map).env).xdf2.recs.offset((line2 - 1) as isize);
        (*record1).ha == (*record2).ha
    }
}

unsafe fn walk_common_sequence(
    map: *mut HashMap,
    mut first: *mut Entry,
    mut line1: c_int,
    count1: c_int,
    mut line2: c_int,
    count2: c_int,
) -> c_int {
    let end1 = line1 + count1;
    let end2 = line2 + count2;

    unsafe {
        loop {
            let mut next1: c_int;
            let mut next2: c_int;
            if !first.is_null() {
                next1 = (*first).line1 as c_int;
                next2 = (*first).line2 as c_int;
                while next1 > line1 && next2 > line2 && match_lines(map, next1 - 1, next2 - 1) {
                    next1 -= 1;
                    next2 -= 1;
                }
            } else {
                next1 = end1;
                next2 = end2;
            }
            while line1 < next1 && line2 < next2 && match_lines(map, line1, line2) {
                line1 += 1;
                line2 += 1;
            }

            if (next1 > line1 || next2 > line2)
                && patience_diff(
                    (*map).file1,
                    (*map).file2,
                    (*map).xpp,
                    (*map).env,
                    line1,
                    next1 - line1,
                    line2,
                    next2 - line2,
                ) != 0
            {
                return -1;
            }

            if first.is_null() {
                return 0;
            }

            while !(*first).next.is_null()
                && (*(*first).next).line1 == (*first).line1 + 1
                && (*(*first).next).line2 == (*first).line2 + 1
            {
                first = (*first).next;
            }

            line1 = ((*first).line1 + 1) as c_int;
            line2 = ((*first).line2 + 1) as c_int;

            first = (*first).next;
        }
    }
}

unsafe fn fall_back_to_classic_diff(
    map: *mut HashMap,
    line1: c_int,
    count1: c_int,
    line2: c_int,
    count2: c_int,
) -> c_int {
    unsafe {
        let xpp = xpparam_t {
            flags: (*(*map).xpp).flags & !XDF_DIFF_ALGORITHM_MASK,
            anchors: ptr::null_mut(),
            anchors_nr: 0,
        };
        xdl_fall_back_diff((*map).env, &xpp, line1, count1, line2, count2)
    }
}

unsafe fn patience_diff(
    file1: *mut mmfile_t,
    file2: *mut mmfile_t,
    xpp: *const xpparam_t,
    env: *mut xdfenv_t,
    mut line1: c_int,
    mut count1: c_int,
    mut line2: c_int,
    mut count2: c_int,
) -> c_int {
    unsafe {
        if count1 == 0 {
            while count2 > 0 {
                count2 -= 1;
                *(*env).xdf2.rchg.offset((line2 - 1) as isize) = 1;
                line2 += 1;
            }
            return 0;
        } else if count2 == 0 {
            while count1 > 0 {
                count1 -= 1;
                *(*env).xdf1.rchg.offset((line1 - 1) as isize) = 1;
                line1 += 1;
            }
            return 0;
        }

        let mut map: HashMap = zeroed();
        if fill_hashmap(
            file1, file2, xpp, env, &mut map, line1, count1, line2, count2,
        ) != 0
        {
            return -1;
        }

        if map.has_matches == 0 {
            while count1 > 0 {
                count1 -= 1;
                *(*env).xdf1.rchg.offset((line1 - 1) as isize) = 1;
                line1 += 1;
            }
            while count2 > 0 {
                count2 -= 1;
                *(*env).xdf2.rchg.offset((line2 - 1) as isize) = 1;
                line2 += 1;
            }
            xfree(map.entries.cast::<c_void>());
            return 0;
        }

        let first = find_longest_common_sequence(&mut map);
        let result = if !first.is_null() {
            walk_common_sequence(&mut map, first, line1, count1, line2, count2)
        } else {
            fall_back_to_classic_diff(&mut map, line1, count1, line2, count2)
        };

        xfree(map.entries.cast::<c_void>());
        result
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn xdl_do_patience_diff(
    file1: *mut mmfile_t,
    file2: *mut mmfile_t,
    xpp: *const xpparam_t,
    env: *mut xdfenv_t,
) -> c_int {
    unsafe {
        if xdl_prepare_env(file1, file2, xpp, env) < 0 {
            return -1;
        }

        patience_diff(
            file1,
            file2,
            xpp,
            env,
            1,
            (*env).xdf1.nrec as c_int,
            1,
            (*env).xdf2.nrec as c_int,
        )
    }
}
