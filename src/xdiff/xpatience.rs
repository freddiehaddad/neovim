//! Patience diff algorithm (`src/xdiff/xpatience.c`).

use crate::xdiff::xprepare::xdl_prepare_env;
use crate::xdiff::xutils::xdl_fall_back_diff;
use crate::xdiff::{Mmfile, XDF_DIFF_ALGORITHM_MASK, Xdfenv, Xpparam};

const NON_UNIQUE: u64 = u64::MAX;

#[derive(Copy, Clone, Default)]
struct Entry {
    hash: u64,
    line1: u64,
    line2: u64,
    next: Option<usize>,
    previous: Option<usize>,
}

struct HashMap {
    nr: i32,
    alloc: i32,
    entries: Vec<Entry>,
    first: Option<usize>,
    last: Option<usize>,
    has_matches: u64,
}

fn insert_record(line: i32, map: &mut HashMap, env: &Xdfenv<'_>, pass: i32) {
    let records = if pass == 1 {
        &env.xdf1.recs
    } else {
        &env.xdf2.recs
    };
    let record = &records[(line - 1) as usize];
    let mut index = ((record.ha << 1) % (map.alloc as u64)) as usize;

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
            map.entries[index].line2 = line as u64;
        }
        return;
    }
    if pass == 2 {
        return;
    }
    map.entries[index].line1 = line as u64;
    map.entries[index].hash = record.ha;
    if map.first.is_none() {
        map.first = Some(index);
    }
    if let Some(last_idx) = map.last {
        map.entries[last_idx].next = Some(index);
        map.entries[index].previous = Some(last_idx);
    }
    map.last = Some(index);
    map.nr += 1;
}

fn fill_hashmap(
    env: &Xdfenv<'_>,
    mut line1: i32,
    mut count1: i32,
    mut line2: i32,
    mut count2: i32,
) -> HashMap {
    let alloc = count1 * 2;
    let mut map = HashMap {
        nr: 0,
        alloc,
        entries: vec![Entry::default(); alloc as usize],
        first: None,
        last: None,
        has_matches: 0,
    };

    while count1 > 0 {
        count1 -= 1;
        insert_record(line1, &mut map, env, 1);
        line1 += 1;
    }

    while count2 > 0 {
        count2 -= 1;
        insert_record(line2, &mut map, env, 2);
        line2 += 1;
    }
    map
}

fn binary_search(entries: &[Entry], sequence: &[usize], longest: i32, entry: &Entry) -> i32 {
    let mut left: i32 = -1;
    let mut right: i32 = longest;

    while left + 1 < right {
        let middle = left + (right - left) / 2;
        if entries[sequence[middle as usize]].line2 > entry.line2 {
            right = middle;
        } else {
            left = middle;
        }
    }
    left
}

fn find_longest_common_sequence(map: &mut HashMap) -> Option<usize> {
    let mut sequence = vec![0usize; map.nr as usize];

    let mut longest: i32 = 0;
    let mut entry_opt = map.first;

    while let Some(entry_idx) = entry_opt {
        let ent = map.entries[entry_idx];
        if ent.line2 != 0 && ent.line2 != NON_UNIQUE {
            let mut i = binary_search(&map.entries, &sequence, longest, &ent);
            map.entries[entry_idx].previous = if i < 0 {
                None
            } else {
                Some(sequence[i as usize])
            };
            i += 1;
            sequence[i as usize] = entry_idx;
            if i == longest {
                longest += 1;
            }
        }
        entry_opt = ent.next;
    }

    if longest == 0 {
        return None;
    }

    let mut entry_idx = sequence[(longest - 1) as usize];
    map.entries[entry_idx].next = None;
    while let Some(prev_idx) = map.entries[entry_idx].previous {
        map.entries[prev_idx].next = Some(entry_idx);
        entry_idx = prev_idx;
    }
    Some(entry_idx)
}

fn match_lines(env: &Xdfenv<'_>, line1: i32, line2: i32) -> bool {
    let record1 = &env.xdf1.recs[(line1 - 1) as usize];
    let record2 = &env.xdf2.recs[(line2 - 1) as usize];
    record1.ha == record2.ha
}

fn walk_common_sequence<'a>(
    file1: &mut Mmfile<'a>,
    file2: &mut Mmfile<'a>,
    xpp: &Xpparam,
    env: &mut Xdfenv<'a>,
    map: &HashMap,
    mut first: Option<usize>,
    mut line1: i32,
    count1: i32,
    mut line2: i32,
    count2: i32,
) -> i32 {
    let end1 = line1 + count1;
    let end2 = line2 + count2;

    loop {
        let mut next1: i32;
        let mut next2: i32;
        if let Some(first_idx) = first {
            let first_ent = &map.entries[first_idx];
            next1 = first_ent.line1 as i32;
            next2 = first_ent.line2 as i32;
            while next1 > line1 && next2 > line2 && match_lines(env, next1 - 1, next2 - 1) {
                next1 -= 1;
                next2 -= 1;
            }
        } else {
            next1 = end1;
            next2 = end2;
        }
        while line1 < next1 && line2 < next2 && match_lines(env, line1, line2) {
            line1 += 1;
            line2 += 1;
        }

        if (next1 > line1 || next2 > line2)
            && patience_diff(
                file1,
                file2,
                xpp,
                env,
                line1,
                next1 - line1,
                line2,
                next2 - line2,
            ) != 0
        {
            return -1;
        }

        let Some(mut first_idx) = first else {
            return 0;
        };

        while let Some(next_idx) = map.entries[first_idx].next
            && map.entries[next_idx].line1 == map.entries[first_idx].line1 + 1
            && map.entries[next_idx].line2 == map.entries[first_idx].line2 + 1
        {
            first_idx = next_idx;
        }

        line1 = (map.entries[first_idx].line1 + 1) as i32;
        line2 = (map.entries[first_idx].line2 + 1) as i32;

        first = map.entries[first_idx].next;
    }
}

fn fall_back_to_classic_diff(
    xpp: &Xpparam,
    env: &mut Xdfenv<'_>,
    line1: i32,
    count1: i32,
    line2: i32,
    count2: i32,
) -> i32 {
    let xpp = Xpparam {
        flags: xpp.flags & !XDF_DIFF_ALGORITHM_MASK,
    };
    xdl_fall_back_diff(env, &xpp, line1, count1, line2, count2)
}

fn patience_diff<'a>(
    file1: &mut Mmfile<'a>,
    file2: &mut Mmfile<'a>,
    xpp: &Xpparam,
    env: &mut Xdfenv<'a>,
    mut line1: i32,
    mut count1: i32,
    mut line2: i32,
    mut count2: i32,
) -> i32 {
    if count1 == 0 {
        while count2 > 0 {
            count2 -= 1;
            env.xdf2.set_rchg(i64::from(line2 - 1), 1);
            line2 += 1;
        }
        return 0;
    } else if count2 == 0 {
        while count1 > 0 {
            count1 -= 1;
            env.xdf1.set_rchg(i64::from(line1 - 1), 1);
            line1 += 1;
        }
        return 0;
    }

    let mut map = fill_hashmap(env, line1, count1, line2, count2);

    if map.has_matches == 0 {
        while count1 > 0 {
            count1 -= 1;
            env.xdf1.set_rchg(i64::from(line1 - 1), 1);
            line1 += 1;
        }
        while count2 > 0 {
            count2 -= 1;
            env.xdf2.set_rchg(i64::from(line2 - 1), 1);
            line2 += 1;
        }
        return 0;
    }

    let first = find_longest_common_sequence(&mut map);
    if first.is_some() {
        walk_common_sequence(
            file1, file2, xpp, env, &map, first, line1, count1, line2, count2,
        )
    } else {
        fall_back_to_classic_diff(xpp, env, line1, count1, line2, count2)
    }
}

pub fn xdl_do_patience_diff<'a>(
    file1: &mut Mmfile<'a>,
    file2: &mut Mmfile<'a>,
    xpp: &Xpparam,
    env: &mut Xdfenv<'a>,
) -> i32 {
    if xdl_prepare_env(file1, file2, xpp, env) < 0 {
        return -1;
    }

    patience_diff(
        file1,
        file2,
        xpp,
        env,
        1,
        env.xdf1.nrec as i32,
        1,
        env.xdf2.nrec as i32,
    )
}
