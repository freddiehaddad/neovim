use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

type RowDef = (&'static str, &'static str, &'static str, fn(f64) -> String);

const BENCH_SPECS: &[&str] = &[
    "test/benchmark/autocmd_spec.lua",
    "test/benchmark/bench_regexp_spec.lua",
    "test/benchmark/decor_spec.lua",
    "test/benchmark/extmark_spec.lua",
    "test/benchmark/option_spec.lua",
    "test/benchmark/screenpos_spec.lua",
    "test/benchmark/search_spec.lua",
    "test/benchmark/sort_spec.lua",
    "test/benchmark/treesitter_spec.lua",
];

pub fn resolve_upstream_sha(root: &Path, explicit_rev: Option<&str>) -> String {
    if let Some(rev) = explicit_rev
        && let Ok(out) = Command::new("git")
            .current_dir(root)
            .args([
                "rev-parse",
                "--verify",
                "--short=10",
                &format!("{rev}^{{commit}}"),
            ])
            .output()
        && out.status.success()
    {
        return String::from_utf8_lossy(&out.stdout).trim().to_string();
    }
    let rev = explicit_rev.unwrap_or("master");
    let status = Command::new("git")
        .current_dir(root)
        .args(["fetch", "https://github.com/neovim/neovim.git", rev])
        .status()
        .expect("failed to fetch upstream neovim");
    assert!(status.success(), "failed to fetch upstream {rev}");

    let out = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "--short=10", "FETCH_HEAD"])
        .output()
        .expect("failed to resolve FETCH_HEAD");
    assert!(out.status.success(), "git rev-parse FETCH_HEAD failed");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for ch in chars.by_ref() {
                if ch.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn extract_after_prefix_f64(text: &str, prefix: &str) -> Option<f64> {
    let idx = text.find(prefix)? + prefix.len();
    let rest = text[idx..].trim_start();
    let num_len = rest
        .bytes()
        .take_while(|b| b.is_ascii_digit() || *b == b'.')
        .count();
    rest.get(..num_len)?.parse().ok()
}

fn extract_before_suffix_f64(text: &str, anchor: &str, suffix: &str) -> Option<f64> {
    let start = text.find(anchor)? + anchor.len();
    let sub = &text[start..];
    let end = sub.find(suffix)?;
    let before = sub[..end].trim_end();
    let num_start = before
        .rfind(|c: char| !c.is_ascii_digit() && c != '.')
        .map_or(0, |i| i + 1);
    before[num_start..].parse().ok()
}

fn extract_third_csv_stat(text: &str, anchor: &str, unit: &str) -> Option<f64> {
    let start = text.find(anchor)? + anchor.len();
    let sub = &text[start..];
    let line_anchor = "min, 25%, median, 75%, max:";
    let pos = sub.find(line_anchor)? + line_anchor.len();
    let next_line = sub[pos..].lines().nth(1)?;
    let third = next_line.split(',').nth(2)?.trim();
    third.strip_suffix(unit)?.trim().parse().ok()
}

fn extract_section<'a>(text: &'a str, spec_name: &str) -> Option<&'a str> {
    let start_marker = format!("Running tests from {spec_name}");
    let end_marker = format!("tests from {spec_name}");
    let start = text.find(&start_marker)? + start_marker.len();
    let rest = &text[start..];
    let end = rest.find(&end_marker)?;
    Some(&rest[..end])
}

fn extract_bench_report_median(text: &str, label: &str) -> Option<f64> {
    let needle = format!("\n{label}\n");
    let start = text.find(&needle)? + needle.len();
    let line = text[start..].lines().next()?;
    extract_after_prefix_f64(line, "median")
}

fn sum_autocmd_hrtime_ms(text: &str) -> Option<f64> {
    let section = extract_section(text, "test/benchmark/autocmd_spec.lua")?;
    let mut sum = 0.0;
    let mut count = 0usize;
    for line in section.lines() {
        let Some(end) = line.find(" ms - ") else {
            continue;
        };
        let before = line[..end].trim_end();
        let num_start = before
            .rfind(|c: char| !c.is_ascii_digit() && c != '.')
            .map_or(0, |i| i + 1);
        let val: f64 = before[num_start..].parse().ok()?;
        sum += val;
        count += 1;
    }
    (count == 12).then_some(sum)
}

fn sum_option_medians_ms(text: &str) -> Option<f64> {
    let section = extract_section(text, "test/benchmark/option_spec.lua")?;
    let mut sum = 0.0;
    let mut count = 0usize;
    for line in section.lines() {
        if line.contains("(ms, n=20000)")
            && let Some(med) = extract_after_prefix_f64(line, "median")
        {
            sum += med * 20_000.0;
            count += 1;
        }
    }
    (count == 7).then_some(sum)
}

fn sum_csv_medians(section: &str, unit: &str, expected_count: usize) -> Option<f64> {
    let line_anchor = "min, 25%, median, 75%, max:";
    let mut rest = section;
    let mut sum = 0.0;
    let mut count = 0usize;
    while let Some(pos) = rest.find(line_anchor) {
        let after = &rest[pos + line_anchor.len()..];
        let next_line = after.lines().nth(1)?;
        let third = next_line.split(',').nth(2)?.trim();
        let val: f64 = third.strip_suffix(unit)?.trim().parse().ok()?;
        sum += val;
        count += 1;
        rest = after;
    }
    (count == expected_count).then_some(sum)
}

pub fn parse_benchmark_text(raw_text: &str) -> BTreeMap<&'static str, f64> {
    let clean = strip_ansi(raw_text).replace("\r\n", "\n");
    let mut m = BTreeMap::new();

    if let Some(v) = extract_after_prefix_f64(&clean, "median=") {
        m.insert("startup_ms", v);
    }
    if let Some(v) = sum_autocmd_hrtime_ms(&clean) {
        m.insert("autocmd_suite_ms", v);
    }
    if let Some(v) = extract_before_suffix_f64(
        &clean,
        "nvim_create_autocmd, nvim_del_autocmd (unique patterns):",
        " ms - nvim_del_autocmd",
    ) {
        m.insert("autocmd_del_unique_ms", v);
    }
    if let Some(v) = extract_after_prefix_f64(&clean, "re: 0, time:") {
        m.insert("regexp_auto_ms", v * 1000.0);
    }
    if let Some(v) = extract_after_prefix_f64(&clean, "re: 1, time:") {
        m.insert("regexp_bt_ms", v * 1000.0);
    }
    if let Some(v) = extract_third_csv_stat(&clean, "can handle full screen of highlighting:", "ms")
    {
        m.insert("decor_fullscreen_ms", v);
    }
    if let Some(v) = extract_before_suffix_f64(
        &clean,
        "many non-wrapped virtual lines:",
        " ms - moving cursor down to line 2",
    ) {
        m.insert("extmark_cursor_us", v * 1000.0);
    }
    if let Some(v) = sum_option_medians_ms(&clean) {
        m.insert("option_suite_ms", v);
    }
    if let Some(sec) = extract_section(&clean, "test/benchmark/screenpos_spec.lua")
        && let Some(sum_us) = sum_csv_medians(sec, "us", 24)
    {
        m.insert("screenpos_suite_s", sum_us / 100.0);
    }
    if let Some(v) = extract_third_csv_stat(&clean, "for long line, ascii, nobreakindent:", "us") {
        m.insert("screenpos_ascii_us", v);
    }
    if let Some(v) =
        extract_third_csv_stat(&clean, "for long line, 3 byte utf-8, nobreakindent:", "us")
    {
        m.insert("screenpos_utf8_us", v);
    }
    if let Some(v) = extract_bench_report_median(&clean, "vimgrep_100k") {
        m.insert("vimgrep_100k_ms", v);
    }
    if let Some(v) = extract_bench_report_median(&clean, "matchfuzzy_50k") {
        m.insert("matchfuzzy_50k_ms", v);
    }
    if let Some(v) = extract_bench_report_median(&clean, "diff_linematch") {
        m.insert("diff_linematch_ms", v);
    }
    if let Some(v) = extract_bench_report_median(&clean, "sort") {
        m.insert("sort_lines_ms", v);
    }
    if let Some(v) = extract_bench_report_median(&clean, r"sort /\w\+ /") {
        m.insert("sort_pattern_ms", v);
    }
    if let Some(v) = extract_after_prefix_f64(&clean, "treesitter fold:") {
        m.insert("treesitter_fold_ms", v);
    }
    if let Some(sec) = extract_section(&clean, "test/benchmark/treesitter_spec.lua")
        && let Some(sum_ms) = sum_csv_medians(sec, "ms", 7)
    {
        m.insert("treesitter_suite_ms", sum_ms * 100.0);
    }

    m
}

pub fn parse_benchmark_log(path: &Path) -> BTreeMap<&'static str, f64> {
    let raw_bytes =
        fs::read(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let text = if raw_bytes.starts_with(&[0xFF, 0xFE]) {
        let u16s: Vec<u16> = raw_bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .copied()
            .map(u16::from_le_bytes)
            .collect();
        String::from_utf16_lossy(&u16s)
    } else {
        String::from_utf8_lossy(&raw_bytes).into_owned()
    };
    parse_benchmark_text(&text)
}

fn median_metric(runs: &[BTreeMap<&'static str, f64>], key: &'static str) -> f64 {
    let mut vals: Vec<f64> = runs
        .iter()
        .map(|r| {
            *r.get(key)
                .unwrap_or_else(|| panic!("missing benchmark metric '{key}' in parsed log"))
        })
        .collect();
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    vals[vals.len() / 2]
}

fn build_windows_upstream_c(root: &Path, target_dir: &Path, sha: &str) -> PathBuf {
    let worktree = target_dir.join("bench-upstream-c");
    let exe_path = worktree.join("build/bin/nvim.exe");
    if exe_path.exists()
        && Command::new("git")
            .current_dir(&worktree)
            .args(["rev-parse", "--short=10", "HEAD"])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == sha)
    {
        return exe_path;
    }
    if worktree.exists() {
        let _ = Command::new("git")
            .current_dir(root)
            .args(["worktree", "remove", "--force"])
            .arg(&worktree)
            .status();
        let _ = fs::remove_dir_all(&worktree);
    }
    let status = Command::new("git")
        .current_dir(root)
        .args(["worktree", "add", "--detach"])
        .arg(&worktree)
        .arg(sha)
        .status()
        .expect("failed to create upstream worktree");
    assert!(status.success(), "git worktree add failed for {sha}");

    let downloads_src = root.join(".deps").join("build").join("downloads");
    let downloads_dst = worktree.join(".deps").join("build").join("downloads");
    if downloads_src.is_dir() {
        let _ = Command::new("robocopy")
            .args(["/E", "/NFL", "/NDL", "/NJH", "/NJS", "/NC", "/NS"])
            .arg(&downloads_src)
            .arg(&downloads_dst)
            .status();
    }

    let vcvars = [
        r"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files\Microsoft Visual Studio\2022\Enterprise\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat",
    ]
    .into_iter()
    .find(|p| Path::new(p).exists())
    .expect("could not locate vcvars64.bat for Windows C Release build");

    let bat_script = format!(
        "@echo off\r\nset \"PATH=%PATH%;C:\\Program Files (x86)\\Microsoft Visual Studio\\Installer\"\r\ncall \"{vcvars}\" >nul || exit /b 1\r\ncmake -S cmake.deps -B .deps -G Ninja -DCMAKE_BUILD_TYPE=Release || exit /b 1\r\ncmake --build .deps || exit /b 1\r\ncmake -B build -G Ninja -DCMAKE_BUILD_TYPE=Release || exit /b 1\r\ncmake --build build || exit /b 1\r\n"
    );
    let bat_path = worktree.join("build_upstream.bat");
    fs::write(&bat_path, bat_script).expect("failed to write build_upstream.bat");
    let status = Command::new("cmd.exe")
        .current_dir(&worktree)
        .env_remove("NoDefaultCurrentDirectoryInExePath")
        .args(["/c", "build_upstream.bat"])
        .status()
        .expect("failed to build Windows upstream C");
    assert!(status.success(), "Windows upstream C Release build failed");

    let parser_src = worktree.join(".deps/usr/lib/nvim/parser");
    let parser_dst = worktree.join("build/lib/nvim/parser");
    if parser_src.is_dir() {
        fs::create_dir_all(&parser_dst).unwrap();
        for entry in fs::read_dir(&parser_src).unwrap().flatten() {
            let _ = fs::copy(entry.path(), parser_dst.join(entry.file_name()));
        }
    }

    worktree.join("build/bin/nvim.exe")
}

fn run_windows_rounds(root: &Path, log_dir: &Path, upstream_bin: &Path, rounds: usize) {
    let exe = std::env::current_exe().expect("failed to get current xtask exe");
    for round in 1..=rounds {
        println!("[benchmark] Windows Round {round}/{rounds}: Upstream C (Release)...");
        let c_log = fs::File::create(log_dir.join(format!("bench_win_c_{round}.txt"))).unwrap();
        let c_err = c_log.try_clone().unwrap();
        let status = Command::new(&exe)
            .current_dir(root)
            .arg("benchmark")
            .arg("--nvim-bin")
            .arg(upstream_bin)
            .args(BENCH_SPECS)
            .stdout(c_log)
            .stderr(c_err)
            .status()
            .expect("failed to run Windows C benchmark");
        assert!(status.success(), "Windows C benchmark round {round} failed");

        println!("[benchmark] Windows Round {round}/{rounds}: Rust Port (--release)...");
        let r_log = fs::File::create(log_dir.join(format!("bench_win_rust_{round}.txt"))).unwrap();
        let r_err = r_log.try_clone().unwrap();
        let status = Command::new(&exe)
            .current_dir(root)
            .args(["benchmark", "--release"])
            .args(BENCH_SPECS)
            .stdout(r_log)
            .stderr(r_err)
            .status()
            .expect("failed to run Windows Rust benchmark");
        assert!(
            status.success(),
            "Windows Rust benchmark round {round} failed"
        );
    }
}

fn to_wsl_path(win_path: &Path) -> String {
    let s = win_path.to_string_lossy().replace('\\', "/");
    if let Some((drive, rest)) = s.split_once(':')
        && drive.len() == 1
    {
        return format!("/mnt/{}{}", drive.to_ascii_lowercase(), rest);
    }
    s
}

fn run_linux_rounds_via_wsl(root: &Path, log_dir: &Path, sha: &str, rounds: usize) {
    let wsl_root = to_wsl_path(root);
    let wsl_log_dir = to_wsl_path(log_dir);
    let specs_joined = BENCH_SPECS.join(" ");
    let script = format!(
        r#"set -e
mkdir -p ~/.cache/nvim-bench
rm -rf ~/.cache/nvim-bench/upstream-c ~/.cache/nvim-bench/rust-port
git clone --no-checkout "{wsl_root}" ~/.cache/nvim-bench/upstream-c
git -C ~/.cache/nvim-bench/upstream-c checkout {sha}
if [ -d "{wsl_root}/.deps/build/downloads" ]; then
  mkdir -p ~/.cache/nvim-bench/upstream-c/.deps/build
  cp -r "{wsl_root}/.deps/build/downloads" ~/.cache/nvim-bench/upstream-c/.deps/build/downloads
fi
cd ~/.cache/nvim-bench/upstream-c
cmake -S cmake.deps -B .deps -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build .deps
cmake -B build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build
mkdir -p ~/.cache/nvim-bench/upstream-c/build/lib/nvim/parser
cp -r ~/.cache/nvim-bench/upstream-c/.deps/usr/lib/nvim/parser/* ~/.cache/nvim-bench/upstream-c/build/lib/nvim/parser/

git clone "{wsl_root}" ~/.cache/nvim-bench/rust-port
cp "{wsl_root}/src/xtask/main.rs" ~/.cache/nvim-bench/rust-port/src/xtask/main.rs
cp "{wsl_root}/src/xtask/bench.rs" ~/.cache/nvim-bench/rust-port/src/xtask/bench.rs
cp "{wsl_root}"/test/benchmark/*.lua ~/.cache/nvim-bench/rust-port/test/benchmark/
if [ -d "{wsl_root}/.deps/build/downloads" ]; then
  mkdir -p ~/.cache/nvim-bench/rust-port/.deps/build
  cp -r "{wsl_root}/.deps/build/downloads" ~/.cache/nvim-bench/rust-port/.deps/build/downloads
fi
cd ~/.cache/nvim-bench/rust-port
~/.cargo/bin/cargo xtask build --release

for i in $(seq 1 {rounds}); do
  echo "[benchmark] Linux Round $i/{rounds}: Upstream C (Release)..."
  ~/.cargo/bin/cargo xtask benchmark --nvim-bin ~/.cache/nvim-bench/upstream-c/build/bin/nvim {specs_joined} > "{wsl_log_dir}/bench_linux_c_$i.txt" 2>&1
  echo "[benchmark] Linux Round $i/{rounds}: Rust Port (--release)..."
  ~/.cargo/bin/cargo xtask benchmark --release {specs_joined} > "{wsl_log_dir}/bench_linux_rust_$i.txt" 2>&1
done
rm -rf ~/.cache/nvim-bench
"#
    );

    let status = Command::new("wsl")
        .args(["-e", "bash", "-lc", &script])
        .status()
        .expect("failed to execute WSL for Linux benchmarks");
    assert!(status.success(), "Linux WSL benchmark run failed");
}

pub fn generate_svg(
    sha: &str,
    linux_c: &[BTreeMap<&'static str, f64>],
    linux_rust: &[BTreeMap<&'static str, f64>],
    win_c: &[BTreeMap<&'static str, f64>],
    win_rust: &[BTreeMap<&'static str, f64>],
) -> String {
    let rows: &[RowDef] = &[
        (
            "Cold Startup",
            "Launch &amp; exit headless (25-run median)",
            "startup_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Screen Layout &amp; Wrapping",
            "240,000 cursor &amp; wrap position queries",
            "screenpos_suite_s",
            |v| format!("{v:.1} s"),
        ),
        (
            "Long UTF-8 Line Layout",
            "5,000-character multibyte line median",
            "screenpos_utf8_us",
            |v| format!("{v:.1} µs"),
        ),
        (
            "Multi-Core Buffer Search",
            "Regex scan across 100,000 lines",
            "vimgrep_100k_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Fuzzy File Finder",
            "Score &amp; rank 50,000 project file paths",
            "matchfuzzy_50k_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Side-by-Side Diff Alignment",
            "Myers diff with line-similarity scoring",
            "diff_linematch_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Regex Search (Backtracking)",
            "Complex pattern search on HTML text",
            "regexp_bt_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Virtual Text Cursor Motion",
            "Cursor step over 2,000 virtual lines",
            "extmark_cursor_us",
            |v| format!("{v:.1} µs"),
        ),
        (
            "Treesitter Fold Navigation",
            "100 motions &amp; redraws over folded code",
            "treesitter_fold_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Full-Screen Syntax Redraw",
            "100x50 window with Treesitter highlight",
            "decor_fullscreen_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Autocommand Event Hooks",
            "Create, trigger &amp; delete 82,500 hooks",
            "autocmd_suite_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Editor Option Updates",
            "140,000 vim.o &amp; vim.opt setting updates",
            "option_suite_ms",
            |v| format!("{v:.1} ms"),
        ),
    ];

    let bar_x = 350_i32;
    let max_bar_w = 405.0_f64;
    let base_y = 100_i32;
    let row_h = 82_i32;
    let canvas_h = base_y + (rows.len() as i32) * row_h + 10;
    let frame_h = canvas_h - 16;

    let mut delay_css = String::new();
    for idx in 0..rows.len() {
        let row_num = idx + 1;
        let d_delay = 0.05 + (idx as f64) * 0.07;
        let b_delay = 0.65 + (idx as f64) * 0.07;
        let _ = writeln!(
            delay_css,
            "    .d{row_num} {{ animation-delay: {d_delay:.2}s; }} .b{row_num} {{ animation-delay: {b_delay:.2}s; }}"
        );
    }

    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 960 {canvas_h}" width="100%" role="img" aria-label="Neovim Rust Port vs Upstream C Performance Benchmarks (Linux and Windows)">
  <defs>
    <linearGradient id="backdrop" x2="1" y2="1">
      <stop stop-color="#0a1425"/>
      <stop offset=".55" stop-color="#0b1727"/>
      <stop offset="1" stop-color="#13192c"/>
    </linearGradient>
    <radialGradient id="ambient">
      <stop stop-color="#235a83" stop-opacity=".22"/>
      <stop offset="1" stop-color="#235a83" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="cardFill" x2="0" y2="1">
      <stop stop-color="#13263b" stop-opacity=".65"/>
      <stop offset="1" stop-color="#0c192a" stop-opacity=".8"/>
    </linearGradient>
    <pattern id="dots" width="32" height="32" patternUnits="userSpaceOnUse">
      <circle cx="1" cy="1" r=".8" fill="#7f9fc5" opacity=".09"/>
    </pattern>
    <linearGradient id="rustLinuxBar" x1="0%" y1="0%" x2="100%" y2="0%">
      <stop offset="0%" stop-color="#2f6f5c"/>
      <stop offset="100%" stop-color="#7aebba"/>
    </linearGradient>
    <linearGradient id="cLinuxBar" x1="0%" y1="0%" x2="100%" y2="0%">
      <stop offset="0%" stop-color="#31506b"/>
      <stop offset="100%" stop-color="#6385a6"/>
    </linearGradient>
    <linearGradient id="rustWinBar" x1="0%" y1="0%" x2="100%" y2="0%">
      <stop offset="0%" stop-color="#4379dc"/>
      <stop offset="100%" stop-color="#61c9ff"/>
    </linearGradient>
    <linearGradient id="cWinBar" x1="0%" y1="0%" x2="100%" y2="0%">
      <stop offset="0%" stop-color="#5e6296"/>
      <stop offset="100%" stop-color="#958bff"/>
    </linearGradient>
  </defs>
  <style>
    :root {{
      color-scheme: dark;
      --border: #3a5875;
      --divider: #30465f;
      --text-main: #f1f6ff;
      --text-sub: #a1b3ca;
      --text-dim: #8ea3bc;
      --rust-linux-text: #7aebba;
      --c-linux-text: #8bb5d6;
      --rust-win-text: #61c9ff;
      --c-win-text: #bdb4ff;
      --badge-win-bg: #13342f;
      --badge-win-border: #33675a;
      --badge-win-text: #7aebba;
      --badge-neutral-bg: #30261f;
      --badge-neutral-border: #77503b;
      --badge-neutral-text: #ffbb7c;
      --track: #142236;
    }}
    .canvas {{ fill: url(#backdrop); }}
    .frame {{ fill: url(#cardFill); stroke: var(--border); stroke-width: 1.5; stroke-opacity: 0.8; }}
    .divider {{ stroke: var(--divider); stroke-width: 1; stroke-opacity: 0.55; }}
    .title {{ font: 700 17px "Segoe UI", -apple-system, BlinkMacSystemFont, Helvetica, Arial, sans-serif; fill: var(--text-main); }}
    .subtitle {{ font: 400 12px "Segoe UI", -apple-system, BlinkMacSystemFont, Helvetica, Arial, sans-serif; fill: var(--text-sub); }}
    .legend {{ font: 600 11.5px "Segoe UI", -apple-system, BlinkMacSystemFont, Helvetica, Arial, sans-serif; fill: var(--text-main); }}
    .metric-title {{ font: 600 13.5px "Segoe UI", -apple-system, BlinkMacSystemFont, Helvetica, Arial, sans-serif; fill: var(--text-main); }}
    .metric-desc {{ font: 400 11px "Segoe UI", -apple-system, BlinkMacSystemFont, Helvetica, Arial, sans-serif; fill: var(--text-sub); }}
    .row-tag {{ font: 600 10.5px "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace; fill: var(--text-dim); text-anchor: end; }}
    .val-rl {{ font: 700 11px "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace; fill: var(--rust-linux-text); opacity: 0; animation: fadeIn 0.35s ease-out forwards; }}
    .val-cl {{ font: 500 11px "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace; fill: var(--c-linux-text); opacity: 0; animation: fadeIn 0.35s ease-out forwards; }}
    .val-rw {{ font: 700 11px "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace; fill: var(--rust-win-text); opacity: 0; animation: fadeIn 0.35s ease-out forwards; }}
    .val-cw {{ font: 500 11px "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace; fill: var(--c-win-text); opacity: 0; animation: fadeIn 0.35s ease-out forwards; }}
    .badge-neutral {{ fill: var(--badge-neutral-bg); stroke: var(--badge-neutral-border); stroke-width: 1.2; }}
    .badge-win {{ fill: var(--badge-win-bg); stroke: var(--badge-win-border); stroke-width: 1.2; animation: pillWin 0.45s ease-out forwards; }}
    .badge-label-win {{ font: 700 11px "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace; fill: var(--badge-win-text); text-anchor: middle; opacity: 0; animation: fadeIn 0.45s ease-out forwards; }}
    .badge-label-neutral {{ font: 700 11px "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace; fill: var(--badge-neutral-text); text-anchor: middle; opacity: 0; animation: fadeIn 0.45s ease-out forwards; }}
    .track {{ fill: var(--track); stroke: #243449; stroke-width: 0.6; }}

    @keyframes grow {{
      0% {{ transform: scaleX(0); }}
      100% {{ transform: scaleX(1); }}
    }}
    @keyframes fadeIn {{
      0% {{ opacity: 0; transform: translateY(2px); }}
      100% {{ opacity: 1; transform: translateY(0); }}
    }}
    @keyframes pillWin {{
      0% {{ fill: var(--badge-neutral-bg); stroke: var(--badge-neutral-border); }}
      100% {{ fill: var(--badge-win-bg); stroke: var(--badge-win-border); }}
    }}
    .bar {{
      transform-box: fill-box;
      transform-origin: left center;
      transform: scaleX(0);
      animation: grow 0.75s cubic-bezier(0.22, 1, 0.36, 1) forwards;
    }}
{delay_css}  </style>

  <rect class="canvas" width="960" height="{canvas_h}" rx="12"/>
  <rect width="960" height="{canvas_h}" rx="12" fill="url(#dots)"/>
  <ellipse cx="630" cy="480" rx="520" ry="420" fill="url(#ambient)"/>
  <rect class="frame" x="8" y="8" width="944" height="{frame_h}" rx="10"/>

  <!-- Header -->
  <text class="title" x="28" y="38">Head-to-Head Editor Benchmarks: Rust Port vs. Upstream C (neovim@{sha})</text>
  <text class="subtitle" x="28" y="58">Release build execution time (shorter bars = faster; badges show Rust speedup over C).</text>

  <!-- Legend -->
  <rect x="28" y="70" width="12" height="12" rx="3" fill="url(#rustLinuxBar)"/>
  <text class="legend" x="46" y="80">Rust / Linux (rustc)</text>
  <rect x="210" y="70" width="12" height="12" rx="3" fill="url(#cLinuxBar)"/>
  <text class="legend" x="228" y="80">Upstream C / Linux (GCC)</text>
  <rect x="415" y="70" width="12" height="12" rx="3" fill="url(#rustWinBar)"/>
  <text class="legend" x="433" y="80">Rust / Windows (rustc)</text>
  <rect x="615" y="70" width="12" height="12" rx="3" fill="url(#cWinBar)"/>
  <text class="legend" x="633" y="80">Upstream C / Windows (MSVC)</text>

  <line class="divider" x1="28" y1="92" x2="932" y2="92"/>
"##
    );

    for (idx, &(title, desc, key, fmt_fn)) in rows.iter().enumerate() {
        let y = base_y + (idx as i32) * row_h;
        let rl = median_metric(linux_rust, key);
        let cl = median_metric(linux_c, key);
        let rw = median_metric(win_rust, key);
        let cw = median_metric(win_c, key);
        let max_val = rl.max(cl).max(rw).max(cw);

        let w_rl = ((rl / max_val * max_bar_w).round() as i32).max(6);
        let w_cl = ((cl / max_val * max_bar_w).round() as i32).max(6);
        let w_rw = ((rw / max_val * max_bar_w).round() as i32).max(6);
        let w_cw = ((cw / max_val * max_bar_w).round() as i32).max(6);

        let spd_l = cl / rl;
        let spd_w = cw / rw;
        let cls_l = if spd_l >= 1.0 { "win" } else { "neutral" };
        let cls_w = if spd_w >= 1.0 { "win" } else { "neutral" };

        let row_num = idx + 1;
        let _ = write!(
            svg,
            r#"  <!-- Row {row_num}: {} -->
  <g transform="translate(0, {y})">
    <text class="metric-title" x="28" y="28">{title}</text>
    <text class="metric-desc" x="28" y="46">{desc}</text>

    <text class="row-tag" x="340" y="15">Rust / Linux</text>
    <rect class="track" x="{bar_x}" y="5" width="405" height="11" rx="3"/>
    <rect class="bar d{row_num}" x="{bar_x}" y="5" width="{w_rl}" height="11" rx="3" fill="url(#rustLinuxBar)"/>
    <text class="val-rl b{row_num}" x="{}" y="14">{}</text>

    <text class="row-tag" x="340" y="31">C / Linux</text>
    <rect class="track" x="{bar_x}" y="21" width="405" height="11" rx="3"/>
    <rect class="bar d{row_num}" x="{bar_x}" y="21" width="{w_cl}" height="11" rx="3" fill="url(#cLinuxBar)"/>
    <text class="val-cl b{row_num}" x="{}" y="30">{}</text>

    <text class="row-tag" x="340" y="47">Rust / Win</text>
    <rect class="track" x="{bar_x}" y="37" width="405" height="11" rx="3"/>
    <rect class="bar d{row_num}" x="{bar_x}" y="37" width="{w_rw}" height="11" rx="3" fill="url(#rustWinBar)"/>
    <text class="val-rw b{row_num}" x="{}" y="46">{}</text>

    <text class="row-tag" x="340" y="63">C / Win</text>
    <rect class="track" x="{bar_x}" y="53" width="405" height="11" rx="3"/>
    <rect class="bar d{row_num}" x="{bar_x}" y="53" width="{w_cw}" height="11" rx="3" fill="url(#cWinBar)"/>
    <text class="val-cw b{row_num}" x="{}" y="62">{}</text>

    <rect class="badge-{cls_l} b{row_num}" x="826" y="8" width="106" height="24" rx="12"/>
    <text class="badge-label-{cls_l} b{row_num}" x="879" y="24">Linux: {spd_l:.2}x</text>
    <rect class="badge-{cls_w} b{row_num}" x="826" y="38" width="106" height="24" rx="12"/>
    <text class="badge-label-{cls_w} b{row_num}" x="879" y="54">Win: {spd_w:.2}x</text>
  </g>
"#,
            title.replace("&amp;", "&"),
            bar_x + w_rl + 8,
            fmt_fn(rl),
            bar_x + w_cl + 8,
            fmt_fn(cl),
            bar_x + w_rw + 8,
            fmt_fn(rw),
            bar_x + w_cw + 8,
            fmt_fn(cw),
        );

        if idx + 1 < rows.len() {
            let _ = writeln!(
                svg,
                "  <line class=\"divider\" x1=\"28\" y1=\"{}\" x2=\"932\" y2=\"{}\"/>",
                y + 74,
                y + 74
            );
        }
    }

    svg.push_str("</svg>\n");
    svg
}

pub fn generate_readme_perf_section(sha: &str) -> String {
    format!(
        r#"## Performance

Measured head-to-head against release builds of upstream [`neovim/neovim@{sha}`](https://github.com/neovim/neovim/commit/{sha}) on the same hardware across **Linux** (GCC vs. `rustc`) and **Windows** (MSVC vs. `rustc`), reporting the median of 3 interleaved runs:

<p align="center">
  <img src="contrib/benchmarks.svg" width="100%" alt="Performance comparison between the Rust port and upstream C Neovim on Linux and Windows">
</p>

<details>
<summary><strong>Benchmark methodology</strong></summary>

- **Speedup**: Relative speedup of the Rust port over upstream C (`C time ÷ Rust time`; values above `1.00x` mean Rust is faster, and below `1.00x` mean C is faster).
- **In-process timing**: Editor operations are timed inside the running editor with high-resolution timers (`vim.uv.hrtime()` / `reltime()`) so test-runner startup is never mixed into workload measurements.
- **Same commit & hardware**: Both builds are compiled from the same synced upstream commit (`MAINTAIN.md`) on the same machine and filesystem (`ext4` on Linux; NTFS on Windows).
- **Release optimization**: Upstream C is built with `-DCMAKE_BUILD_TYPE=Release` (`-O3 -DNDEBUG` on GCC; `/O2 /Ob2 /DNDEBUG` on MSVC), and the Rust port is built with `cargo xtask build --release` (`opt-level = 3`, `lto = "thin"`).
- **Reproducible runner**: Run `cargo xtask benchmark --compare --update-readme` to execute 3 interleaved rounds across both platforms and regenerate `contrib/benchmarks.svg`.

</details>
"#
    )
}

pub fn format_console_summary(
    sha: &str,
    linux_c: &[BTreeMap<&'static str, f64>],
    linux_rust: &[BTreeMap<&'static str, f64>],
    win_c: &[BTreeMap<&'static str, f64>],
    win_rust: &[BTreeMap<&'static str, f64>],
) -> String {
    let table_rows: &[RowDef] = &[
        (
            "Cold startup (25-run median)",
            "Startup",
            "startup_ms",
            |v| format!("{v:.2} ms"),
        ),
        (
            "Screen layout & wrapping (240k queries)",
            "Layout",
            "screenpos_suite_s",
            |v| format!("{v:.2} s"),
        ),
        (
            "Long ASCII line layout (median)",
            "Layout",
            "screenpos_ascii_us",
            |v| format!("{v:.2} µs"),
        ),
        (
            "Long UTF-8 line layout (median)",
            "Layout",
            "screenpos_utf8_us",
            |v| format!("{v:.2} µs"),
        ),
        (
            "Multi-core buffer search (:vimgrep 100k)",
            "Search",
            "vimgrep_100k_ms",
            |v| format!("{v:.2} ms"),
        ),
        (
            "Regex search (backtracking)",
            "Search",
            "regexp_bt_ms",
            |v| format!("{v:.2} ms"),
        ),
        ("Regex search (NFA auto)", "Search", "regexp_auto_ms", |v| {
            format!("{v:.2} ms")
        }),
        (
            "Fuzzy file finder (matchfuzzy 50k)",
            "Picker",
            "matchfuzzy_50k_ms",
            |v| format!("{v:.2} ms"),
        ),
        (
            "Large file sorting (:sort 200k)",
            "Editing",
            "sort_lines_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Regex-keyed sorting (:sort /\\w\\+ / 200k)",
            "Editing",
            "sort_pattern_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Side-by-side diff alignment (linematch)",
            "Diff",
            "diff_linematch_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Virtual text cursor motion (2k lines)",
            "Display",
            "extmark_cursor_us",
            |v| format!("{v:.2} µs"),
        ),
        (
            "Treesitter fold navigation (100 motions)",
            "Syntax",
            "treesitter_fold_ms",
            |v| format!("{v:.2} ms"),
        ),
        (
            "Treesitter long-line redraws (700 redraws)",
            "Syntax",
            "treesitter_suite_ms",
            |v| format!("{v:.0} ms"),
        ),
        (
            "Full-screen syntax redraw (100x50)",
            "Syntax",
            "decor_fullscreen_ms",
            |v| format!("{v:.2} ms"),
        ),
        (
            "Autocommand event hooks (82.5k ops)",
            "Plugins",
            "autocmd_suite_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Delete 7,500 unique autocommands",
            "Plugins",
            "autocmd_del_unique_ms",
            |v| format!("{v:.2} ms"),
        ),
        (
            "Editor option updates (140k calls)",
            "Config",
            "option_suite_ms",
            |v| format!("{v:.1} ms"),
        ),
    ];

    let mut out = format!(
        "Benchmark summary against neovim@{sha}:\n\
         | Workload | Category | Linux C | Linux Rust | Speedup | Windows C | Windows Rust | Speedup |\n\
         |---|---|---:|---:|---:|---:|---:|---:|\n"
    );
    for &(workload, category, key, fmt_fn) in table_rows {
        let lc = median_metric(linux_c, key);
        let lr = median_metric(linux_rust, key);
        let wc = median_metric(win_c, key);
        let wr = median_metric(win_rust, key);
        let _ = writeln!(
            out,
            "| {workload} | {category} | {} | {} | {:.2}x | {} | {} | {:.2}x |",
            fmt_fn(lc),
            fmt_fn(lr),
            lc / lr,
            fmt_fn(wc),
            fmt_fn(wr),
            wc / wr,
        );
    }
    out
}

pub fn run_compare_benchmarks(
    root: &Path,
    target_dir: &Path,
    rounds: usize,
    upstream_rev: Option<&str>,
    update_readme: bool,
    from_logs: Option<&Path>,
) {
    let sha = resolve_upstream_sha(root, upstream_rev);
    let log_dir = from_logs
        .map(PathBuf::from)
        .unwrap_or_else(|| target_dir.join("bench-logs"));
    fs::create_dir_all(&log_dir).unwrap();

    if from_logs.is_none() {
        println!(
            "[benchmark] Comparing against upstream commit neovim@{sha} ({rounds} interleaved rounds on Windows + Linux)..."
        );

        if cfg!(target_os = "windows") {
            let upstream_win_bin = build_windows_upstream_c(root, target_dir, &sha);
            run_windows_rounds(root, &log_dir, &upstream_win_bin, rounds);
            let worktree = target_dir.join("bench-upstream-c");
            let _ = Command::new("git")
                .current_dir(root)
                .args(["worktree", "remove", "--force"])
                .arg(&worktree)
                .status();
            run_linux_rounds_via_wsl(root, &log_dir, &sha, rounds);
        } else {
            panic!(
                "--compare full Windows+Linux orchestration must be invoked from the Windows host with WSL enabled, or pass --from-logs <dir>"
            );
        }
    }

    let load_group = |prefix: &str| -> Vec<BTreeMap<&'static str, f64>> {
        (1..=rounds)
            .map(|i| parse_benchmark_log(&log_dir.join(format!("{prefix}_{i}.txt"))))
            .collect()
    };

    let linux_c = load_group("bench_linux_c");
    let linux_rust = load_group("bench_linux_rust");
    let win_c = load_group("bench_win_c");
    let win_rust = load_group("bench_win_rust");

    let summary = format_console_summary(&sha, &linux_c, &linux_rust, &win_c, &win_rust);
    println!("\n{summary}");

    if update_readme {
        let perf_section = generate_readme_perf_section(&sha);
        let svg = generate_svg(&sha, &linux_c, &linux_rust, &win_c, &win_rust);
        let svg_path = root.join("contrib").join("benchmarks.svg");
        fs::write(&svg_path, svg).expect("failed to write contrib/benchmarks.svg");

        let readme_path = root.join("README.md");
        let readme = fs::read_to_string(&readme_path).expect("failed to read README.md");
        let start_idx = readme
            .find("## Performance\n")
            .expect("could not find '## Performance' in README.md");
        let end_idx = readme[start_idx..]
            .find("\n## Quick Start\n")
            .expect("could not find '## Quick Start' after '## Performance' in README.md")
            + start_idx;
        let mut updated_readme = String::with_capacity(readme.len());
        updated_readme.push_str(&readme[..start_idx]);
        updated_readme.push_str(&perf_section);
        updated_readme.push_str(&readme[end_idx..]);
        fs::write(&readme_path, updated_readme).expect("failed to write README.md");
        println!(
            "[benchmark] Updated {} and {} for upstream commit {sha}",
            svg_path.display(),
            readme_path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_benchmark_text_uses_in_process_timings_not_busted_totals() {
        let mut log = String::new();
        log.push_str(
            "-------- Cold startup benchmark (nvim --clean --headless +q, 25 runs): min=4.10ms, p25=4.20ms, median=5.80ms, p75=6.00ms, max=7.00ms\n",
        );
        log.push_str("-------- Running tests from test/benchmark/autocmd_spec.lua\n");
        log.push_str("RUN T1 autocmd perf nvim_create_autocmd, nvim_del_autocmd (same pattern): 3.000000 ms - nvim_create_autocmd\n 0.500000 ms - nvim_del_autocmd\n");
        log.push_str("RUN T2 autocmd perf nvim_create_autocmd, nvim_del_autocmd (unique patterns): 10.000000 ms - nvim_create_autocmd\n 1.250000 ms - nvim_del_autocmd\n");
        for _ in 0..8 {
            log.push_str("  2.000000 ms - op\n");
        }
        log.push_str("-------- 8 tests from test/benchmark/autocmd_spec.lua (9999.00 ms total)\n");

        log.push_str(
            "file: test/old/testdir/samples/re.freeze.txt, re: 0, time:   0.004000\n\
             file: test/old/testdir/samples/re.freeze.txt, re: 1, time:   0.002000\n",
        );
        log.push_str(
            "RUN T15 decor perf can handle full screen of highlighting:\n\
             Total min, 25%, median, 75%, max:\n\t14.0ms,\t14.5ms,\t15.0ms,\t15.5ms,\t19.0ms\n",
        );
        log.push_str(
            "RUN T21 extmark perf many non-wrapped virtual lines: 3.000000 ms - redraw with virt lines at the bottom\n\
             0.035000 ms - moving cursor down to line 2\n",
        );

        log.push_str("-------- Running tests from test/benchmark/option_spec.lua\n");
        for _ in 0..7 {
            log.push_str(
                "  min 0.0004  q25 0.0004  median 0.0010  q75 0.0010  max 0.0100  mean 0.0010  (ms, n=20000)\n",
            );
        }
        log.push_str("-------- 7 tests from test/benchmark/option_spec.lua (8888.00 ms total)\n");

        log.push_str("-------- Running tests from test/benchmark/screenpos_spec.lua\n");
        log.push_str(
            "RUN T30 screenpos() perf for long line, ascii, nobreakindent: min, 25%, median, 75%, max:\n\t1.40us,\t1.45us,\t1.50us,\t1.55us,\t2.00us\n",
        );
        log.push_str(
            "RUN T34 screenpos() perf for long line, 3 byte utf-8, nobreakindent: min, 25%, median, 75%, max:\n\t70.00us,\t75.00us,\t80.00us,\t85.00us,\t90.00us\n",
        );
        for _ in 0..22 {
            log.push_str(
                "RUN T99 screenpos() perf for other: min, 25%, median, 75%, max:\n\t50.00us,\t50.00us,\t50.00us,\t50.00us,\t50.00us\n",
            );
        }
        log.push_str(
            "-------- 24 tests from test/benchmark/screenpos_spec.lua (77777.00 ms total)\n",
        );

        log.push_str(
            "\nvimgrep_100k\n  min 19.0000  q25 19.5000  median 20.2500  q75 21.0000  max 22.0000  mean 20.3000  (ms, n=5)\n\
             \nmatchfuzzy_50k\n  min 9.0000  q25 9.1000  median 9.4000  q75 9.5000  max 9.8000  mean 9.3500  (ms, n=5)\n\
             \ndiff_linematch\n  min 105.0000  q25 108.0000  median 110.5000  q75 112.0000  max 115.0000  mean 110.1000  (ms, n=5)\n\
             \nsort\n  min 220.0000  q25 225.0000  median 230.0000  q75 235.0000  max 240.0000  mean 230.0000  (ms, n=5)\n\
             \nsort /\\w\\+ /\n  min 310.0000  q25 315.0000  median 320.0000  q75 325.0000  max 330.0000  mean 320.0000  (ms, n=5)\n",
        );

        log.push_str("-------- Running tests from test/benchmark/treesitter_spec.lua\n");
        log.push_str("treesitter fold: 7.25 ms\n");
        for _ in 0..7 {
            log.push_str(
                "Total min, 25%, median, 75%, max:\n\t0.40ms,\t0.45ms,\t0.50ms,\t0.55ms,\t1.00ms\n",
            );
        }
        log.push_str(
            "-------- 8 tests from test/benchmark/treesitter_spec.lua (6666.00 ms total)\n",
        );

        let parsed = parse_benchmark_text(&log);
        assert_eq!(parsed.len(), 18);
        assert!((parsed["autocmd_suite_ms"] - 30.75).abs() < 1e-6);
        assert!((parsed["autocmd_del_unique_ms"] - 1.25).abs() < 1e-6);
        assert!((parsed["option_suite_ms"] - 140.0).abs() < 1e-6);
        assert!((parsed["screenpos_suite_s"] - 11.815).abs() < 1e-6);
        assert!((parsed["treesitter_fold_ms"] - 7.25).abs() < 1e-6);
        assert!((parsed["treesitter_suite_ms"] - 350.0).abs() < 1e-6);
        assert!((parsed["vimgrep_100k_ms"] - 20.25).abs() < 1e-6);
        assert!((parsed["matchfuzzy_50k_ms"] - 9.40).abs() < 1e-6);
        assert!((parsed["diff_linematch_ms"] - 110.50).abs() < 1e-6);
        assert!((parsed["sort_lines_ms"] - 230.0).abs() < 1e-6);
        assert!((parsed["sort_pattern_ms"] - 320.0).abs() < 1e-6);

        let runs = vec![parsed];
        let svg = generate_svg("d9a842e291", &runs, &runs, &runs, &runs);
        assert!(svg.contains("viewBox=\"0 0 960 1094\""));
        assert!(svg.contains(".d12 { animation-delay:"));
        assert!(svg.contains(".b12 { animation-delay:"));
        assert!(!svg.contains("screenpos_spec.lua"));
        assert!(!svg.contains("autocmd_spec.lua"));
        assert!(!svg.contains("option_spec.lua"));
    }
}
