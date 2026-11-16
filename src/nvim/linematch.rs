//! Diff `linematch` dynamic-programming alignment algorithm (`src/nvim/linematch.c`).
#![forbid(unsafe_code)]

const LN_MAX_BUFS: usize = 8;
const LN_DECISION_MAX: usize = (1 << LN_MAX_BUFS) - 1;
const MATCH_CHAR_MAX_LEN: usize = 800;

use crate::cursor::Linenr;

use crate::xdiff::Mmfile;

struct DiffCmpPath {
    df_lev_score: i32,
    df_path_n: usize,
    df_choice_mem: [i32; LN_DECISION_MAX + 1],
    df_choice: [i32; LN_DECISION_MAX],
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
fn line_slice<'a>(m: &Mmfile<'a>) -> &'a [u8] {
    let raw = m.slice;
    let len = raw.iter().position(|&b| b == b'\n').unwrap_or(raw.len());
    &raw[..len.min(MATCH_CHAR_MAX_LEN - 1)]
}

fn matching_chars_slice(s1: &[u8], s2: &[u8]) -> i32 {
    let s1len = s1.len().min(MATCH_CHAR_MAX_LEN - 1);
    let s2len = s2.len().min(MATCH_CHAR_MAX_LEN - 1);
    let mut matrix = [[0_i32; MATCH_CHAR_MAX_LEN]; 2];
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

/// Advance `s` to line `lnum` (1-based) (`fastforward_buf_to_lnum`).
pub fn fastforward_buf_to_lnum<'a>(mut s: Mmfile<'a>, lnum: Linenr) -> Mmfile<'a> {
    for _ in 0..(lnum - 1) {
        if let Some(pos) = s.slice.iter().position(|&b| b == b'\n') {
            s.slice = &s.slice[pos + 1..];
        } else {
            s.slice = &[];
            break;
        }
    }
    s
}

fn unwrap_indexes(values: &[i32], diff_len: &[i32]) -> usize {
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

struct PrecomputedLinematch {
    non_empty: [Vec<bool>; LN_MAX_BUFS],
    pair_scores: [[Vec<i32>; LN_MAX_BUFS]; LN_MAX_BUFS],
}

impl PrecomputedLinematch {
    fn build(diff_blk: &[Mmfile<'_>], diff_len: &[i32], iwhite: bool) -> Self {
        let ndiffs = diff_len.len();
        let mut non_empty: [Vec<bool>; LN_MAX_BUFS] = Default::default();
        let mut lines: [Vec<&[u8]>; LN_MAX_BUFS] = Default::default();
        let mut stripped: [Vec<Vec<u8>>; LN_MAX_BUFS] = Default::default();

        for k in 0..ndiffs {
            let dl = diff_len[k] as usize;
            non_empty[k].reserve(dl);
            lines[k].reserve(dl);
            if iwhite {
                stripped[k].reserve(dl);
            }
            let mut cur = diff_blk[k];
            for _ in 0..dl {
                non_empty[k].push(!cur.slice.is_empty());
                let ls = line_slice(&cur);
                lines[k].push(ls);
                if iwhite {
                    let mut p = Vec::with_capacity(ls.len());
                    for &b in ls {
                        if b != b' ' && b != b'\t' {
                            p.push(b);
                        }
                    }
                    stripped[k].push(p);
                }
                if let Some(pos) = cur.slice.iter().position(|&b| b == b'\n') {
                    cur.slice = &cur.slice[pos + 1..];
                } else {
                    cur.slice = &[];
                }
            }
        }

        let mut pair_scores: [[Vec<i32>; LN_MAX_BUFS]; LN_MAX_BUFS] = Default::default();
        for i in 0..ndiffs {
            let len_i = diff_len[i] as usize;
            for j in (i + 1)..ndiffs {
                let len_j = diff_len[j] as usize;
                let total_pairs = len_i * len_j;
                if total_pairs == 0 {
                    continue;
                }
                let lines_i = &lines[i];
                let lines_j = &lines[j];
                let strip_i = &stripped[i];
                let strip_j = &stripped[j];
                let ne_i = &non_empty[i];
                let ne_j = &non_empty[j];

                if total_pairs >= 1024 && len_i >= 8 {
                    let nthreads = std::thread::available_parallelism()
                        .map_or(4, |n| n.get().min(8))
                        .min(len_i);
                    let rows_per_worker = len_i.div_ceil(nthreads).max(1);
                    let scores = std::thread::scope(|s| {
                        let mut handles = Vec::with_capacity(nthreads);
                        for chunk_start in (0..len_i).step_by(rows_per_worker) {
                            let chunk_end = (chunk_start + rows_per_worker).min(len_i);
                            handles.push(s.spawn(move || {
                                let mut sub = Vec::with_capacity((chunk_end - chunk_start) * len_j);
                                for li in chunk_start..chunk_end {
                                    for lj in 0..len_j {
                                        let sc = if ne_i[li] && ne_j[lj] {
                                            if iwhite {
                                                matching_chars_slice(&strip_i[li], &strip_j[lj])
                                            } else {
                                                matching_chars_slice(lines_i[li], lines_j[lj])
                                            }
                                        } else {
                                            0
                                        };
                                        sub.push(sc);
                                    }
                                }
                                sub
                            }));
                        }
                        let mut combined = Vec::with_capacity(total_pairs);
                        for h in handles {
                            if let Ok(mut part) = h.join() {
                                combined.append(&mut part);
                            }
                        }
                        combined
                    });
                    pair_scores[i][j] = scores;
                } else {
                    let mut scores = Vec::with_capacity(total_pairs);
                    for li in 0..len_i {
                        for lj in 0..len_j {
                            let sc = if ne_i[li] && ne_j[lj] {
                                if iwhite {
                                    matching_chars_slice(&strip_i[li], &strip_j[lj])
                                } else {
                                    matching_chars_slice(lines_i[li], lines_j[lj])
                                }
                            } else {
                                0
                            };
                            scores.push(sc);
                        }
                    }
                    pair_scores[i][j] = scores;
                }
            }
        }

        Self {
            non_empty,
            pair_scores,
        }
    }

    #[inline]
    fn score_choice(&self, df_iters: &[i32], diff_len: &[i32], choice: i32) -> i32 {
        let ndiffs = diff_len.len();
        let mut matched_chars: i32 = 0;
        let mut matched: i32 = 0;

        for i in 0..ndiffs {
            if (choice & (1 << i)) == 0 {
                continue;
            }
            let li = (df_iters[i] - 1) as usize;
            if !self.non_empty[i][li] {
                continue;
            }
            for j in (i + 1)..ndiffs {
                if (choice & (1 << j)) == 0 {
                    continue;
                }
                let lj = (df_iters[j] - 1) as usize;
                if !self.non_empty[j][lj] {
                    continue;
                }
                matched += 1;
                let len_j = diff_len[j] as usize;
                matched_chars += self.pair_scores[i][j][li * len_j + lj];
            }
        }

        if matched >= 2 {
            matched_chars = (matched_chars * 2) / matched;
        }
        matched_chars
    }
}

fn try_possible_paths(
    df_iters: &[i32],
    paths: &[usize],
    path_idx: usize,
    choice: &mut i32,
    diffcmppath: &mut [DiffCmpPath],
    diff_len: &[i32],
    pre: &PrecomputedLinematch,
) {
    let ndiffs = diff_len.len();
    if path_idx == paths.len() {
        if *choice > 0 {
            let mut from_vals = [0_i32; LN_MAX_BUFS];

            for k in 0..ndiffs {
                from_vals[k] = df_iters[k] - i32::from((*choice & (1 << k)) != 0);
            }

            let unwrapped_idx_from = unwrap_indexes(&from_vals[..ndiffs], diff_len);
            let unwrapped_idx_to = unwrap_indexes(df_iters, diff_len);
            let matched_chars = pre.score_choice(df_iters, diff_len, *choice);
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
    try_possible_paths(
        df_iters,
        paths,
        path_idx + 1,
        choice,
        diffcmppath,
        diff_len,
        pre,
    );
    *choice &= !(1 << bit_place);
    try_possible_paths(
        df_iters,
        paths,
        path_idx + 1,
        choice,
        diffcmppath,
        diff_len,
        pre,
    );
}

fn populate_tensor(
    df_iters: &mut [i32; LN_MAX_BUFS],
    ch_dim: usize,
    diffcmppath: &mut [DiffCmpPath],
    diff_len: &[i32],
    pre: &PrecomputedLinematch,
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
        let mut choice: i32 = 0;
        let unwrapped_idx_to = unwrap_indexes(&df_iters[..ndiffs], diff_len);
        diffcmppath[unwrapped_idx_to].df_lev_score = -1;
        try_possible_paths(
            &df_iters[..ndiffs],
            &paths[..npaths],
            0,
            &mut choice,
            diffcmppath,
            diff_len,
            pre,
        );
        return;
    }

    for i in 0..=diff_len[ch_dim] {
        df_iters[ch_dim] = i;
        populate_tensor(df_iters, ch_dim + 1, diffcmppath, diff_len, pre);
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
            diffcmppath[node_idx].df_choice_mem[lastdecision] = minimum_turns as i32;
        }
    }
    diffcmppath[node_idx].df_choice_mem[lastdecision] as usize
}

/// Find an optimal alignment of lines across `diff_blk` diff buffers (`linematch_nbuffers`).
///
/// # Safety
/// Each `Mmfile` in `diff_blk` with a non-null `ptr` and `size > 0` must be valid for `size`
/// bytes.
pub fn linematch_nbuffers(diff_blk: &[&Mmfile<'_>], diff_len: &[i32], iwhite: bool) -> Vec<i32> {
    let ndiffs = diff_blk.len();
    assert_eq!(ndiffs, diff_len.len());
    assert!(ndiffs <= LN_MAX_BUFS);

    let mut blks = [Mmfile::default(); LN_MAX_BUFS];
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

    let pre = PrecomputedLinematch::build(&blks[..ndiffs], diff_len, iwhite);
    let mut df_iters = [0_i32; LN_MAX_BUFS];
    populate_tensor(&mut df_iters, 0, &mut diffcmppath, diff_len, &pre);

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

    #[test]
    fn fastforward_and_two_buffer_linematch() {
        let buf1 = b"alpha\nbeta_modified\ngamma\n";
        let buf2 = b"unrelated\nbeta_mod\ngamma\n";

        let mm1 = Mmfile { slice: buf1 };
        let mm2 = Mmfile { slice: buf2 };

        let line2 = fastforward_buf_to_lnum(mm1, 2);
        assert!(!line2.slice.is_empty());
        assert_eq!(line_slice(&line2), b"beta_modified");

        let decisions = linematch_nbuffers(&[&mm1, &mm2], &[3, 3], false);
        // All 3 lines paired (1 | 2 == 3)
        assert_eq!(decisions, vec![3, 3, 3]);
    }

    #[test]
    fn linematch_multi_buffer_large_parallel_and_iwhite() {
        let mut s1 = String::new();
        let mut s2 = String::new();
        let mut s3 = String::new();
        for i in 1..=16 {
            s1.push_str(&format!("  common_prefix_line_{i:02} = value_{i};\n"));
            s2.push_str(&format!("common_prefix_line_{i:02} = value_{i};\n"));
            s3.push_str(&format!("\tcommon_prefix_line_{i:02} = value_{i};\n"));
        }
        let mm1 = Mmfile {
            slice: s1.as_bytes(),
        };
        let mm2 = Mmfile {
            slice: s2.as_bytes(),
        };
        let mm3 = Mmfile {
            slice: s3.as_bytes(),
        };
        // 16 * 16 = 256 pairs per buffer pair (single-threaded PrecomputedLinematch path)
        let dec_no_iwhite = linematch_nbuffers(&[&mm1, &mm2, &mm3], &[16, 16, 16], false);
        let dec_iwhite = linematch_nbuffers(&[&mm1, &mm2, &mm3], &[16, 16, 16], true);
        assert_eq!(dec_no_iwhite, vec![7; 16]);
        assert_eq!(dec_iwhite, vec![7; 16]);

        // 34 * 34 = 1,156 pairs (>= 1,024 threshold for parallel PrecomputedLinematch::build)
        let mut p1 = String::new();
        let mut p2 = String::new();
        for i in 1..=34 {
            p1.push_str(&format!("  parallel_linematch_row_{i:02} = {i};\n"));
            p2.push_str(&format!("\tparallel_linematch_row_{i:02} = {i};\n"));
        }
        let mmp1 = Mmfile {
            slice: p1.as_bytes(),
        };
        let mmp2 = Mmfile {
            slice: p2.as_bytes(),
        };
        assert_eq!(
            linematch_nbuffers(&[&mmp1, &mmp2], &[34, 34], false),
            vec![3; 34]
        );
        assert_eq!(
            linematch_nbuffers(&[&mmp1, &mmp2], &[34, 34], true),
            vec![3; 34]
        );
    }
}
