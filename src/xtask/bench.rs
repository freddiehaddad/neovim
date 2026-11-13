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
    let clean = strip_ansi(&text);
    let mut m = BTreeMap::new();

    if let Some(v) = extract_after_prefix_f64(&clean, "median=") {
        m.insert("startup_ms", v);
    }
    if let Some(v) = extract_after_prefix_f64(&clean, "test/benchmark/autocmd_spec.lua (") {
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
    if let Some(v) = extract_after_prefix_f64(&clean, "test/benchmark/extmark_spec.lua (") {
        m.insert("extmark_suite_ms", v);
    }
    if let Some(v) = extract_before_suffix_f64(
        &clean,
        "many non-wrapped virtual lines:",
        " ms - moving cursor down to line 2",
    ) {
        m.insert("extmark_cursor_us", v * 1000.0);
    }
    if let Some(v) = extract_after_prefix_f64(&clean, "test/benchmark/option_spec.lua (") {
        m.insert("option_suite_ms", v);
    }
    if let Some(v) = extract_after_prefix_f64(&clean, "test/benchmark/screenpos_spec.lua (") {
        m.insert("screenpos_suite_s", v / 1000.0);
    }
    if let Some(v) = extract_third_csv_stat(&clean, "for long line, ascii, nobreakindent:", "us") {
        m.insert("screenpos_ascii_us", v);
    }
    if let Some(v) =
        extract_third_csv_stat(&clean, "for long line, 3 byte utf-8, nobreakindent:", "us")
    {
        m.insert("screenpos_utf8_us", v);
    }
    if let Some(v) = extract_after_prefix_f64(&clean, "treesitter perf can handle large folds:") {
        m.insert("treesitter_fold_ms", v);
    }
    if let Some(v) = extract_after_prefix_f64(&clean, "test/benchmark/treesitter_spec.lua (") {
        m.insert("treesitter_suite_ms", v);
    }

    m
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
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 960 930" width="100%" role="img" aria-label="Neovim Rust Port vs Upstream C Performance Benchmarks (Linux and Windows)">
  <defs>
    <linearGradient id="rustLinuxBar" x1="0%" y1="0%" x2="100%" y2="0%">
      <stop offset="0%" stop-color="#059669"/>
      <stop offset="100%" stop-color="#34d399"/>
    </linearGradient>
    <linearGradient id="cLinuxBar" x1="0%" y1="0%" x2="100%" y2="0%">
      <stop offset="0%" stop-color="#475569"/>
      <stop offset="100%" stop-color="#64748b"/>
    </linearGradient>
    <linearGradient id="rustWinBar" x1="0%" y1="0%" x2="100%" y2="0%">
      <stop offset="0%" stop-color="#0284c7"/>
      <stop offset="100%" stop-color="#38bdf8"/>
    </linearGradient>
    <linearGradient id="cWinBar" x1="0%" y1="0%" x2="100%" y2="0%">
      <stop offset="0%" stop-color="#6366f1"/>
      <stop offset="100%" stop-color="#818cf8"/>
    </linearGradient>
  </defs>
  <style>
    :root {{
      --bg: #0d1117;
      --card: #161b22;
      --border: #30363d;
      --text-main: #e6edf3;
      --text-sub: #8b949e;
      --text-dim: #6e7681;
      --rust-linux-text: #34d399;
      --c-linux-text: #94a3b8;
      --rust-win-text: #38bdf8;
      --c-win-text: #a5b4fc;
      --badge-win-bg: rgba(16, 185, 129, 0.14);
      --badge-win-border: rgba(52, 211, 153, 0.35);
      --badge-win-text: #34d399;
      --badge-neutral-bg: rgba(148, 163, 184, 0.12);
      --badge-neutral-border: rgba(148, 163, 184, 0.30);
      --badge-neutral-text: #cbd5e1;
      --track: #21262d;
    }}
    @media (prefers-color-scheme: light) {{
      :root {{
        --bg: #ffffff;
        --card: #f6f8fa;
        --border: #d0d7de;
        --text-main: #1f2328;
        --text-sub: #57606a;
        --text-dim: #6e7781;
        --rust-linux-text: #047857;
        --c-linux-text: #475569;
        --rust-win-text: #0369a1;
        --c-win-text: #4f46e5;
        --badge-win-bg: rgba(16, 185, 129, 0.12);
        --badge-win-border: rgba(5, 150, 105, 0.35);
        --badge-win-text: #047857;
        --badge-neutral-bg: rgba(100, 116, 139, 0.10);
        --badge-neutral-border: rgba(100, 116, 139, 0.30);
        --badge-neutral-text: #334155;
        --track: #eaeef2;
      }}
    }}
    .canvas {{ fill: var(--bg); }}
    .frame {{ fill: var(--card); stroke: var(--border); stroke-width: 1.2; }}
    .divider {{ stroke: var(--border); stroke-width: 1; stroke-opacity: 0.55; }}
    .title {{ font: 700 17px -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; fill: var(--text-main); }}
    .subtitle {{ font: 400 12px -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; fill: var(--text-sub); }}
    .legend {{ font: 600 11.5px -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; fill: var(--text-main); }}
    .metric-title {{ font: 600 13.5px -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; fill: var(--text-main); }}
    .metric-desc {{ font: 400 11px -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; fill: var(--text-sub); }}
    .row-tag {{ font: 600 10.5px "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace; fill: var(--text-sub); text-anchor: end; }}
    .val-rl {{ font: 700 11px "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace; fill: var(--rust-linux-text); opacity: 0; animation: fadeIn 0.35s ease-out forwards; }}
    .val-cl {{ font: 500 11px "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace; fill: var(--c-linux-text); opacity: 0; animation: fadeIn 0.35s ease-out forwards; }}
    .val-rw {{ font: 700 11px "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace; fill: var(--rust-win-text); opacity: 0; animation: fadeIn 0.35s ease-out forwards; }}
    .val-cw {{ font: 500 11px "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace; fill: var(--c-win-text); opacity: 0; animation: fadeIn 0.35s ease-out forwards; }}
    .badge-neutral {{ fill: var(--badge-neutral-bg); stroke: var(--badge-neutral-border); stroke-width: 1; }}
    .badge-win {{ fill: var(--badge-neutral-bg); stroke: var(--badge-neutral-border); stroke-width: 1; animation: pillWin 0.45s ease-out forwards; }}
    .badge-label-win {{ font: 700 11px "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace; fill: var(--badge-win-text); text-anchor: middle; opacity: 0; animation: fadeIn 0.45s ease-out forwards; }}
    .badge-label-neutral {{ font: 700 11px "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace; fill: var(--badge-neutral-text); text-anchor: middle; opacity: 0; animation: fadeIn 0.45s ease-out forwards; }}
    .track {{ fill: var(--track); }}

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
    .d1 {{ animation-delay: 0.05s; }} .b1 {{ animation-delay: 0.65s; }}
    .d2 {{ animation-delay: 0.12s; }} .b2 {{ animation-delay: 0.72s; }}
    .d3 {{ animation-delay: 0.19s; }} .b3 {{ animation-delay: 0.79s; }}
    .d4 {{ animation-delay: 0.26s; }} .b4 {{ animation-delay: 0.86s; }}
    .d5 {{ animation-delay: 0.33s; }} .b5 {{ animation-delay: 0.93s; }}
    .d6 {{ animation-delay: 0.40s; }} .b6 {{ animation-delay: 1.00s; }}
    .d7 {{ animation-delay: 0.47s; }} .b7 {{ animation-delay: 1.07s; }}
    .d8 {{ animation-delay: 0.54s; }} .b8 {{ animation-delay: 1.14s; }}
    .d9 {{ animation-delay: 0.61s; }} .b9 {{ animation-delay: 1.21s; }}
    .d10 {{ animation-delay: 0.68s; }} .b10 {{ animation-delay: 1.28s; }}
  </style>

  <rect class="canvas" width="960" height="930" rx="12"/>
  <rect class="frame" x="8" y="8" width="944" height="914" rx="10"/>

  <!-- Header -->
  <text class="title" x="28" y="38">Head-to-Head Editor Benchmarks: Rust Port vs. Upstream C (neovim@{sha})</text>
  <text class="subtitle" x="28" y="58">Execution time in Release mode (shorter bar = faster; right badges show Rust speedup vs. C).</text>

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

    let rows: &[RowDef] = &[
        (
            "Cold Startup",
            "nvim --clean --headless +q (25-run median)",
            "startup_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Screen Layout &amp; UTF-8 Wrapping",
            "screenpos_spec.lua (24-test suite total)",
            "screenpos_suite_s",
            |v| format!("{v:.1} s"),
        ),
        (
            "Long UTF-8 Line Layout",
            "screenpos_spec.lua (3-byte UTF-8 median)",
            "screenpos_utf8_us",
            |v| format!("{v:.1} µs"),
        ),
        (
            "Regex Search Engine",
            "bench_regexp_spec.lua (backtracking)",
            "regexp_bt_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Virtual Line Cursor Motion",
            "extmark_spec.lua (2,000 virtual lines)",
            "extmark_cursor_us",
            |v| format!("{v:.1} µs"),
        ),
        (
            "Treesitter Fold Navigation",
            "treesitter_spec.lua (100 macro replays)",
            "treesitter_fold_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Plugin Autocmd Lifecycle",
            "autocmd_spec.lua (create/exec/del 10k)",
            "autocmd_suite_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Delete 10k Autocmds",
            "autocmd_spec.lua (unique patterns)",
            "autocmd_del_unique_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Extmarks &amp; Inline Diagnostics",
            "extmark_spec.lua (clear ns + 2k redraw)",
            "extmark_suite_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Option &amp; Buffer Configuration",
            "option_spec.lua (140k vim.o / vim.opt sets)",
            "option_suite_ms",
            |v| format!("{v:.1} ms"),
        ),
    ];

    let bar_x = 350_i32;
    let max_bar_w = 405.0_f64;
    let base_y = 100_i32;
    let row_h = 82_i32;

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

pub fn generate_readme_perf_section(
    sha: &str,
    linux_c: &[BTreeMap<&'static str, f64>],
    linux_rust: &[BTreeMap<&'static str, f64>],
    win_c: &[BTreeMap<&'static str, f64>],
    win_rust: &[BTreeMap<&'static str, f64>],
) -> String {
    let table_rows: &[RowDef] = &[
        (
            "Cold startup (`--clean --headless +q`, 25-run median)",
            "Startup",
            "startup_ms",
            |v| format!("{v:.2} ms"),
        ),
        (
            "Screen layout & wrapping (24 tests)",
            "screenpos",
            "screenpos_suite_s",
            |v| format!("{v:.2} s"),
        ),
        (
            "Long ASCII line layout (median)",
            "screenpos",
            "screenpos_ascii_us",
            |v| format!("{v:.2} µs"),
        ),
        (
            "Long UTF-8 line layout (median)",
            "screenpos",
            "screenpos_utf8_us",
            |v| format!("{v:.2} µs"),
        ),
        (
            "Regex search (backtracking)",
            "regexp",
            "regexp_bt_ms",
            |v| format!("{v:.2} ms"),
        ),
        (
            "Regex search (auto engine)",
            "regexp",
            "regexp_auto_ms",
            |v| format!("{v:.2} ms"),
        ),
        (
            "Cursor motion over 2k virtual lines",
            "extmark",
            "extmark_cursor_us",
            |v| format!("{v:.2} µs"),
        ),
        (
            "Treesitter fold navigation (100 macros)",
            "treesitter",
            "treesitter_fold_ms",
            |v| format!("{v:.2} ms"),
        ),
        (
            "Treesitter redraw suite (8 tests)",
            "treesitter",
            "treesitter_suite_ms",
            |v| format!("{v:.0} ms"),
        ),
        (
            "Autocmd lifecycle suite (10k events)",
            "autocmd",
            "autocmd_suite_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Delete 10k autocmds (unique patterns)",
            "autocmd",
            "autocmd_del_unique_ms",
            |v| format!("{v:.2} ms"),
        ),
        (
            "Extmark & virtual line suite",
            "extmark",
            "extmark_suite_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Option updates (140k calls)",
            "option",
            "option_suite_ms",
            |v| format!("{v:.1} ms"),
        ),
        (
            "Full-screen Treesitter redraw (100x50)",
            "decor",
            "decor_fullscreen_ms",
            |v| format!("{v:.2} ms"),
        ),
    ];

    let mut out = format!(
        r#"## Performance

Measured head-to-head against retail (`CMAKE_BUILD_TYPE=Release`) builds of upstream [`neovim/neovim@{sha}`](https://github.com/neovim/neovim/commit/{sha}) (`v0.13.0-dev`) on identical hardware across **Linux** (GCC `Release` vs. `rustc --release`) and **Windows** (MSVC `Release` vs. `rustc --release`) using 3 interleaved runs via `cargo xtask benchmark --compare`:

<p align="center">
  <img src="contrib/benchmarks.svg" width="100%" alt="Performance comparison between the Rust port and upstream C Neovim on Linux and Windows">
</p>

<details>
<summary><strong>View full benchmark table &amp; methodology</strong></summary>

| Workload | Suite | Linux C | Linux Rust | Speedup | Windows C | Windows Rust | Speedup |
|---|---|---:|---:|---:|---:|---:|---:|
"#
    );

    for &(workload, suite, key, fmt_fn) in table_rows {
        let lc = median_metric(linux_c, key);
        let lr = median_metric(linux_rust, key);
        let wc = median_metric(win_c, key);
        let wr = median_metric(win_rust, key);
        let _ = writeln!(
            out,
            "| {workload} | {suite} | {} | {} | {:.2}x | {} | {} | {:.2}x |",
            fmt_fn(lc),
            fmt_fn(lr),
            lc / lr,
            fmt_fn(wc),
            fmt_fn(wr),
            wc / wr,
        );
    }

    out.push_str(
        r#"
**Methodology**:
- **Speedup**: Relative speedup of the Rust port compared to upstream C (`C time ÷ Rust time`; values `> 1.00x` mean Rust is faster, and `< 1.00x` mean C is faster).
- Both repositories are checked out at the exact same synced upstream commit (`MAINTAIN.md`) on the same filesystem (`ext4` on Linux; NTFS on Windows).
- Upstream C is built from source with `-DCMAKE_BUILD_TYPE=Release` (`-O3 -DNDEBUG` on Linux GCC; `/O2 /Ob2 /DNDEBUG` on Windows MSVC).
- The Rust port is built with `cargo xtask build --release` (`opt-level = 3`, `lto = "thin"`).
- Benchmarks are executed across 3 interleaved rounds (`cargo xtask benchmark --compare --update-readme`) and the median of the 3 rounds is reported.

</details>
"#,
    );
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

    let perf_section = generate_readme_perf_section(&sha, &linux_c, &linux_rust, &win_c, &win_rust);
    println!("\n{perf_section}");

    if update_readme {
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
