//! Patience diff algorithm (`src/xdiff/xpatience.c`).

use std::ffi::{c_int, c_ulong};
use std::ptr;

use crate::xdiff::xprepare::xdl_prepare_env;
use crate::xdiff::xutils::xdl_fall_back_diff;
use crate::xdiff::{XDF_DIFF_ALGORITHM_MASK, Mmfile, Xdfenv, Xpparam};

const NON_UNIQUE: c_ulong = c_ulong::MAX;

#[repr(C)]
#[derive(Copy, Clone)]
struct Entry {
    hash: c_ulong,
    line1: c_ulong,
    line2: c_ulong,
    next: *mut Entry,
    previous: *mut Entry,
}

impl Default for Entry {
    fn default() -> Self {
        Self {
            hash: 0,
            line1: 0,
            line2: 0,
            next: ptr::null_mut(),
            previous: ptr::null_mut(),
        }
    }
}

struct HashMap {
    nr: c_int,
    alloc: c_int,
    entries: Vec<Entry>,
    first: *mut Entry,
    last: *mut Entry,
    has_matches: c_ulong,
    file1: *mut Mmfile,
    file2: *mut Mmfile,
    env: *mut Xdfenv,
    xpp: *const Xpparam,
}

fn insert_record(line: c_int, map: &mut HashMap, pass: c_int) {
    unsafe {
        let records = if pass == 1 {
            &(*map.env).xdf1.recs
        } else {
            &(*map.env).xdf2.recs
        };
        let record = &records[(line - 1) as usize];
        let mut index = ((record.ha << 1) % (map.alloc as c_ulong)) as usize;

        while map.entries[index].line1 != 0 {
            if map.entries[index].hash != record.ha {
                index += 1;
                if index >= map.alloc as usize {
                    index = 0;
                }
                continue;
            }
            if pass == 2 {
                map.has_matches = 1;
            }
            if pass == 1 || map.entries[index].line2 != 0 {
                map.entries[index].line2 = NON_UNIQUE;
            } else {
                map.entries[index].line2 = line as c_ulong;
            }
            return;
        }
        if pass == 2 {
            return;
        }
        let ent = map.entries.as_mut_ptr().add(index);
        (*ent).line1 = line as c_ulong;
        (*ent).hash = record.ha;
        if map.first.is_null() {
            map.first = ent;
        }
        if !map.last.is_null() {
            (*map.last).next = ent;
            (*ent).previous = map.last;
        }
        map.last = ent;
        map.nr += 1;
    }
}

fn fill_hashmap(
    file1: &mut Mmfile,
    file2: &mut Mmfile,
    xpp: &Xpparam,
    env: &mut Xdfenv,
    mut line1: c_int,
    mut count1: c_int,
    mut line2: c_int,
    mut count2: c_int,
) -> HashMap {
    let alloc = count1 * 2;
    let mut map = HashMap {
        nr: 0,
        alloc,
        entries: vec![Entry::default(); alloc as usize],
        first: ptr::null_mut(),
        last: ptr::null_mut(),
        has_matches: 0,
        file1,
        file2,
        env,
        xpp,
    };

    while count1 > 0 {
        count1 -= 1;
        insert_record(line1, &mut map, 1);
        line1 += 1;
    }

    while count2 > 0 {
        count2 -= 1;
        insert_record(line2, &mut map, 2);
        line2 += 1;
    }
    map
}

fn binary_search(sequence: &[*mut Entry], longest: c_int, entry: &Entry) -> c_int {
    let mut left: c_int = -1;
    let mut right: c_int = longest;

    unsafe {
        while left + 1 < right {
            let middle = left + (right - left) / 2;
            if (*sequence[middle as usize]).line2 > entry.line2 {
                right = middle;
            } else {
                left = middle;
            }
        }
    }
    left
}

fn find_longest_common_sequence(map: &mut HashMap) -> *mut Entry {
    unsafe {
        let mut sequence = vec![ptr::null_mut::<Entry>(); map.nr as usize];

        let mut longest: c_int = 0;
        let mut entry = map.first;

        while !entry.is_null() {
            if (*entry).line2 != 0 && (*entry).line2 != NON_UNIQUE {
                let mut i = binary_search(&sequence, longest, &*entry);
                (*entry).previous = if i < 0 {
                    ptr::null_mut()
                } else {
                    sequence[i as usize]
                };
                i += 1;
                sequence[i as usize] = entry;
                if i == longest {
                    longest += 1;
                }
            }
            entry = (*entry).next;
        }

        if longest == 0 {
            return ptr::null_mut();
        }

        entry = sequence[(longest - 1) as usize];
        (*entry).next = ptr::null_mut();
        while !(*entry).previous.is_null() {
            (*(*entry).previous).next = entry;
            entry = (*entry).previous;
        }
        entry
    }
}

fn match_lines(map: &HashMap, line1: c_int, line2: c_int) -> bool {
    unsafe {
        let record1 = &(&(*map.env).xdf1.recs)[(line1 - 1) as usize];
        let record2 = &(&(*map.env).xdf2.recs)[(line2 - 1) as usize];
        record1.ha == record2.ha
    }
}

fn walk_common_sequence(
    map: &mut HashMap,
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
                    &mut *map.file1,
                    &mut *map.file2,
                    &*map.xpp,
                    &mut *map.env,
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

fn fall_back_to_classic_diff(
    map: &HashMap,
    line1: c_int,
    count1: c_int,
    line2: c_int,
    count2: c_int,
) -> c_int {
    unsafe {
        let xpp = Xpparam {
            flags: (*map.xpp).flags & !XDF_DIFF_ALGORITHM_MASK,
        };
        xdl_fall_back_diff(&mut *map.env, &xpp, line1, count1, line2, count2)
    }
}

fn patience_diff(
    file1: &mut Mmfile,
    file2: &mut Mmfile,
    xpp: &Xpparam,
    env: &mut Xdfenv,
    mut line1: c_int,
    mut count1: c_int,
    mut line2: c_int,
    mut count2: c_int,
) -> c_int {
    unsafe {
        if count1 == 0 {
            while count2 > 0 {
                count2 -= 1;
                *env.xdf2.rchg.offset((line2 - 1) as isize) = 1;
                line2 += 1;
            }
            return 0;
        } else if count2 == 0 {
            while count1 > 0 {
                count1 -= 1;
                *env.xdf1.rchg.offset((line1 - 1) as isize) = 1;
                line1 += 1;
            }
            return 0;
        }

        let mut map = fill_hashmap(file1, file2, xpp, env, line1, count1, line2, count2);

        if map.has_matches == 0 {
            while count1 > 0 {
                count1 -= 1;
                *env.xdf1.rchg.offset((line1 - 1) as isize) = 1;
                line1 += 1;
            }
            while count2 > 0 {
                count2 -= 1;
                *env.xdf2.rchg.offset((line2 - 1) as isize) = 1;
                line2 += 1;
            }
            return 0;
        }

        let first = find_longest_common_sequence(&mut map);
        if !first.is_null() {
            walk_common_sequence(&mut map, first, line1, count1, line2, count2)
        } else {
            fall_back_to_classic_diff(&map, line1, count1, line2, count2)
        }
    }
}

pub fn xdl_do_patience_diff(
    file1: &mut Mmfile,
    file2: &mut Mmfile,
    xpp: &Xpparam,
    env: &mut Xdfenv,
) -> c_int {
    if xdl_prepare_env(file1, file2, xpp, env) < 0 {
        return -1;
    }

    patience_diff(
        file1,
        file2,
        xpp,
        env,
        1,
        env.xdf1.nrec as c_int,
        1,
        env.xdf2.nrec as c_int,
    )
}
