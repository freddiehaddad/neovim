//! Diff `linematch` dynamic-programming alignment algorithm (`src/nvim/linematch.c`).

use std::ffi::c_int;

const LN_MAX_BUFS: usize = 8;
const LN_DECISION_MAX: usize = (1 << LN_MAX_BUFS) - 1;
const MATCH_CHAR_MAX_LEN: usize = 800;

use crate::cursor::linenr_T;

use crate::xdiff::mmfile_t;

struct DiffCmpPath {
    df_lev_score: c_int,
    df_path_n: usize,
    df_choice_mem: [c_int; LN_DECISION_MAX + 1],
    df_choice: [c_int; LN_DECISION_MAX],
    df_decision: [usize; LN_DECISION_MAX],
    df_optimal_choice: usize,
}

impl DiffCmpPath {
    fn new(n: usize) -> Self {
        let mut choice_mem = [0; LN_DECISION_MAX + 1];
        for slot in choice_mem.iter_mut().take(n) {
            *slot = -1;
        }
        Self {
            df_lev_score: 0,
            df_path_n: 0,
            df_choice_mem: choice_mem,
            df_choice: [0; LN_DECISION_MAX],
            df_decision: [0; LN_DECISION_MAX],
            df_optimal_choice: 0,
        }
    }
}

#[inline]
unsafe fn line_slice<'a>(m: &mmfile_t) -> &'a [u8] {
    if m.ptr.is_null() || m.size <= 0 {
        return &[];
    }
    let raw = unsafe { std::slice::from_raw_parts(m.ptr.cast::<u8>(), m.size as usize) };
    let len = raw.iter().position(|&b| b == b'\n').unwrap_or(raw.len());
    &raw[..len.min(MATCH_CHAR_MAX_LEN - 1)]
}

fn matching_chars_slice(s1: &[u8], s2: &[u8]) -> c_int {
    let s1len = s1.len().min(MATCH_CHAR_MAX_LEN - 1);
    let s2len = s2.len().min(MATCH_CHAR_MAX_LEN - 1);
    let mut matrix = [[0 as c_int; MATCH_CHAR_MAX_LEN]; 2];
    let mut icur = 1usize;

    for &c1 in &s1[..s1len] {
        icur ^= 1;
        let (row0, row1) = matrix.split_at_mut(1);
        let (e1, e2) = if icur == 0 {
            (&mut row0[0], &row1[0])
        } else {
            (&mut row1[0], &row0[0])
        };

        for (j, &c2) in s2[..s2len].iter().enumerate() {
            let mut best = e1[j + 1];
            if e2[j + 1] > best {
                best = e2[j + 1];
            }
            if e1[j] > best {
                best = e1[j];
            }
            if c1 == c2 && e2[j] + 1 > best {
                best = e2[j] + 1;
            }
            e1[j + 1] = best;
        }
    }

    matrix[icur][s2len]
}

unsafe fn matching_chars_iwhite(s1: &mmfile_t, s2: &mmfile_t) -> c_int {
    let b1 = unsafe { line_slice(s1) };
    let b2 = unsafe { line_slice(s2) };

    let mut p1 = [0u8; MATCH_CHAR_MAX_LEN];
    let mut p2 = [0u8; MATCH_CHAR_MAX_LEN];
    let mut len1 = 0usize;
    let mut len2 = 0usize;

    for &b in b1 {
        if b != b' ' && b != b'\t' {
            p1[len1] = b;
            len1 += 1;
        }
    }
    for &b in b2 {
        if b != b' ' && b != b'\t' {
            p2[len2] = b;
            len2 += 1;
        }
    }

    matching_chars_slice(&p1[..len1], &p2[..len2])
}

unsafe fn matching_chars(m1: &mmfile_t, m2: &mmfile_t) -> c_int {
    let b1 = unsafe { line_slice(m1) };
    let b2 = unsafe { line_slice(m2) };
    matching_chars_slice(b1, b2)
}

unsafe fn count_n_matched_chars(sp: &[mmfile_t], iwhite: bool) -> c_int {
    let mut matched_chars: c_int = 0;
    let mut matched: c_int = 0;
    let n = sp.len();

    for i in 0..n {
        for j in (i + 1)..n {
            if !sp[i].ptr.is_null() && !sp[j].ptr.is_null() {
                matched += 1;
                matched_chars += if iwhite {
                    unsafe { matching_chars_iwhite(&sp[i], &sp[j]) }
                } else {
                    unsafe { matching_chars(&sp[i], &sp[j]) }
                };
            }
        }
    }

    if matched >= 2 {
        matched_chars = (matched_chars * 2) / matched;
    }

    matched_chars
}

/// Advance `s` to line `lnum` (1-based) (`fastforward_buf_to_lnum`).
///
/// # Safety
/// If `s.ptr` is non-null and `s.size > 0`, `s.ptr` must be valid for `s.size` bytes.
pub unsafe fn fastforward_buf_to_lnum(mut s: mmfile_t, lnum: linenr_T) -> mmfile_t {
    for _ in 0..(lnum - 1) {
        if s.ptr.is_null() || s.size <= 0 {
            s.ptr = std::ptr::null_mut();
            s.size = 0;
            break;
        }
        let slice = unsafe { std::slice::from_raw_parts(s.ptr.cast::<u8>(), s.size as usize) };
        if let Some(pos) = slice.iter().position(|&b| b == b'\n') {
            s.ptr = unsafe { s.ptr.add(pos + 1) };
            s.size -= (pos as c_int) + 1;
        } else {
            s.ptr = std::ptr::null_mut();
            s.size = 0;
            break;
        }
    }
    s
}

fn unwrap_indexes(values: &[c_int], diff_len: &[c_int]) -> usize {
    let mut num_unwrap_scalar = 1usize;
    for &dl in diff_len {
        num_unwrap_scalar *= (dl as usize) + 1;
    }

    let mut path_idx = 0usize;
    for (&v, &dl) in values.iter().zip(diff_len.iter()) {
        num_unwrap_scalar /= (dl as usize) + 1;
        path_idx += num_unwrap_scalar * (v as usize);
    }
    path_idx
}

unsafe fn try_possible_paths(
    df_iters: &[c_int],
    paths: &[usize],
    path_idx: usize,
    choice: &mut c_int,
    diffcmppath: &mut [DiffCmpPath],
    diff_len: &[c_int],
    diff_blk: &[mmfile_t],
    iwhite: bool,
) {
    let ndiffs = diff_len.len();
    if path_idx == paths.len() {
        if *choice > 0 {
            let mut from_vals = [0 as c_int; LN_MAX_BUFS];
            let mut mm = [mmfile_t {
                ptr: std::ptr::null_mut(),
                size: 0,
            }; LN_MAX_BUFS];

            for k in 0..ndiffs {
                from_vals[k] = df_iters[k];
                if (*choice & (1 << k)) != 0 {
                    from_vals[k] -= 1;
                    mm[k] = unsafe { fastforward_buf_to_lnum(diff_blk[k], df_iters[k]) };
                }
            }

            let unwrapped_idx_from = unwrap_indexes(&from_vals[..ndiffs], diff_len);
            let unwrapped_idx_to = unwrap_indexes(df_iters, diff_len);
            let matched_chars = unsafe { count_n_matched_chars(&mm[..ndiffs], iwhite) };
            let score = diffcmppath[unwrapped_idx_from].df_lev_score + matched_chars;

            let to_node = &mut diffcmppath[unwrapped_idx_to];
            if score > to_node.df_lev_score {
                to_node.df_path_n = 1;
                to_node.df_decision[0] = unwrapped_idx_from;
                to_node.df_choice[0] = *choice;
                to_node.df_lev_score = score;
            } else if score == to_node.df_lev_score {
                let k = to_node.df_path_n;
                to_node.df_path_n += 1;
                to_node.df_decision[k] = unwrapped_idx_from;
                to_node.df_choice[k] = *choice;
            }
        }
        return;
    }

    let bit_place = paths[path_idx];
    *choice |= 1 << bit_place;
    unsafe {
        try_possible_paths(
            df_iters,
            paths,
            path_idx + 1,
            choice,
            diffcmppath,
            diff_len,
            diff_blk,
            iwhite,
        );
    }
    *choice &= !(1 << bit_place);
    unsafe {
        try_possible_paths(
            df_iters,
            paths,
            path_idx + 1,
            choice,
            diffcmppath,
            diff_len,
            diff_blk,
            iwhite,
        );
    }
}

unsafe fn populate_tensor(
    df_iters: &mut [c_int; LN_MAX_BUFS],
    ch_dim: usize,
    diffcmppath: &mut [DiffCmpPath],
    diff_len: &[c_int],
    diff_blk: &[mmfile_t],
    iwhite: bool,
) {
    let ndiffs = diff_len.len();
    if ch_dim == ndiffs {
        let mut paths = [0usize; LN_MAX_BUFS];
        let mut npaths = 0usize;
        for (j, &iter_val) in df_iters[..ndiffs].iter().enumerate() {
            if iter_val > 0 {
                paths[npaths] = j;
                npaths += 1;
            }
        }
        let mut choice: c_int = 0;
        let unwrapped_idx_to = unwrap_indexes(&df_iters[..ndiffs], diff_len);
        diffcmppath[unwrapped_idx_to].df_lev_score = -1;
        unsafe {
            try_possible_paths(
                &df_iters[..ndiffs],
                &paths[..npaths],
                0,
                &mut choice,
                diffcmppath,
                diff_len,
                diff_blk,
                iwhite,
            );
        }
        return;
    }

    for i in 0..=diff_len[ch_dim] {
        df_iters[ch_dim] = i;
        unsafe {
            populate_tensor(
                df_iters,
                ch_dim + 1,
                diffcmppath,
                diff_len,
                diff_blk,
                iwhite,
            );
        }
    }
}

fn test_charmatch_paths(
    diffcmppath: &mut [DiffCmpPath],
    node_idx: usize,
    lastdecision: usize,
) -> usize {
    if diffcmppath[node_idx].df_choice_mem[lastdecision] == -1 {
        let path_n = diffcmppath[node_idx].df_path_n;
        if path_n == 0 {
            diffcmppath[node_idx].df_choice_mem[lastdecision] = 0;
        } else {
            let mut minimum_turns = usize::MAX;
            let mut best_i = 0usize;
            for i in 0..path_n {
                let next_idx = diffcmppath[node_idx].df_decision[i];
                let choice_i = diffcmppath[node_idx].df_choice[i] as usize;
                let turns = test_charmatch_paths(diffcmppath, next_idx, choice_i)
                    + usize::from(lastdecision != choice_i);
                if turns < minimum_turns {
                    best_i = i;
                    minimum_turns = turns;
                }
            }
            diffcmppath[node_idx].df_optimal_choice = best_i;
            diffcmppath[node_idx].df_choice_mem[lastdecision] = minimum_turns as c_int;
        }
    }
    diffcmppath[node_idx].df_choice_mem[lastdecision] as usize
}

/// Find an optimal alignment of lines across `diff_blk` diff buffers (`linematch_nbuffers`).
///
/// # Safety
/// Each `mmfile_t` in `diff_blk` with a non-null `ptr` and `size > 0` must be valid for `size`
/// bytes.
pub unsafe fn linematch_nbuffers(
    diff_blk: &[&mmfile_t],
    diff_len: &[c_int],
    iwhite: bool,
) -> Vec<c_int> {
    let ndiffs = diff_blk.len();
    assert_eq!(ndiffs, diff_len.len());
    assert!(ndiffs <= LN_MAX_BUFS);

    let mut blks = [mmfile_t {
        ptr: std::ptr::null_mut(),
        size: 0,
    }; LN_MAX_BUFS];
    let mut memsize = 1usize;
    let mut memsize_decisions = 0usize;

    for (i, &dl) in diff_len.iter().enumerate() {
        assert!(dl >= 0);
        memsize *= (dl as usize) + 1;
        memsize_decisions += dl as usize;
        blks[i] = *diff_blk[i];
    }

    let n = 1usize << ndiffs;
    let mut diffcmppath: Vec<DiffCmpPath> = (0..memsize).map(|_| DiffCmpPath::new(n)).collect();

    let mut df_iters = [0 as c_int; LN_MAX_BUFS];
    unsafe {
        populate_tensor(
            &mut df_iters,
            0,
            &mut diffcmppath,
            diff_len,
            &blks[..ndiffs],
            iwhite,
        );
    }

    let u = unwrap_indexes(diff_len, diff_len);
    test_charmatch_paths(&mut diffcmppath, u, 0);

    let mut decisions = Vec::with_capacity(memsize_decisions);
    let mut curr_idx = u;
    while diffcmppath[curr_idx].df_path_n > 0 {
        let j = diffcmppath[curr_idx].df_optimal_choice;
        decisions.push(diffcmppath[curr_idx].df_choice[j]);
        curr_idx = diffcmppath[curr_idx].df_decision[j];
    }

    decisions.reverse();
    decisions
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_char;

    #[test]
    fn fastforward_and_two_buffer_linematch() {
        let buf1 = b"alpha\nbeta_modified\ngamma\n";
        let buf2 = b"unrelated\nbeta_mod\ngamma\n";

        let mm1 = mmfile_t {
            ptr: buf1.as_ptr().cast::<c_char>().cast_mut(),
            size: buf1.len() as c_int,
        };
        let mm2 = mmfile_t {
            ptr: buf2.as_ptr().cast::<c_char>().cast_mut(),
            size: buf2.len() as c_int,
        };

        unsafe {
            let line2 = fastforward_buf_to_lnum(mm1, 2);
            assert!(!line2.ptr.is_null());
            assert_eq!(line_slice(&line2), b"beta_modified");

            let decisions = linematch_nbuffers(&[&mm1, &mm2], &[3, 3], false);
            // All 3 lines paired (1 | 2 == 3)
            assert_eq!(decisions, vec![3, 3, 3]);
        }
    }
}
