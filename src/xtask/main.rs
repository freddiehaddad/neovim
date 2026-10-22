use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

#[path = "../deps_build.rs"]
mod deps_build;

fn exe_suffix() -> &'static str {
    if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    }
}

fn system_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "Darwin"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else if cfg!(target_os = "freebsd") {
        "FreeBSD"
    } else if cfg!(target_os = "openbsd") {
        "OpenBSD"
    } else if cfg!(target_os = "netbsd") {
        "NetBSD"
    } else {
        env::consts::OS
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("src/xtask must be two levels below repo root")
        .to_path_buf()
}

fn default_deps_prefix(root: &Path) -> PathBuf {
    env::var("DEPS_PREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join(".deps").join("usr"))
}

fn cargo_target_dir(root: &Path) -> PathBuf {
    env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("target"))
}

fn run_status(cmd: &mut Command, desc: &str) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to execute {desc}: {e}"));
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn build_cargo_bins(root: &Path, release: bool, prefer_lua: bool) -> PathBuf {
    let deps_prefix = default_deps_prefix(root);
    deps_build::ensure_deps(root, &deps_prefix, prefer_lua, false);

    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut cmd = Command::new(cargo);
    cmd.current_dir(root).arg("build");
    for bin in [
        "nvim",
        "tee",
        "xxd",
        "printargs-test",
        "printenv-test",
        "shell-test",
        "streams-test",
        "tty-test",
    ] {
        cmd.args(["--bin", bin]);
    }
    if release {
        cmd.arg("--release");
    }
    if prefer_lua {
        cmd.args(["--features", "vendor-bit"]);
    }
    run_status(&mut cmd, "cargo build --bins");

    let profile = if release { "release" } else { "debug" };
    cargo_target_dir(root).join(profile)
}

fn stage_build_dir(root: &Path, profile_dir: &Path, build_dir: &Path) -> PathBuf {
    let deps_prefix = default_deps_prefix(root);
    let bin_dir = build_dir.join("bin");
    let lib_nvim_dir = build_dir.join("lib").join("nvim");
    let gen_rt_dir = build_dir.join("runtime");
    fs::create_dir_all(&bin_dir).unwrap();
    fs::create_dir_all(&lib_nvim_dir).unwrap();
    fs::create_dir_all(gen_rt_dir.join("syntax").join("vim")).unwrap();

    let sfx = exe_suffix();
    for bin in [
        "nvim",
        "tee",
        "xxd",
        "printargs-test",
        "printenv-test",
        "shell-test",
        "streams-test",
        "tty-test",
    ] {
        let src = profile_dir.join(format!("{bin}{sfx}"));
        if src.exists() {
            let dst = bin_dir.join(format!("{bin}{sfx}"));
            let _ = fs::copy(&src, &dst);
        }
    }
    let shell_src = profile_dir.join(format!("shell-test{sfx}"));
    if shell_src.exists() {
        let _ = fs::copy(&shell_src, bin_dir.join(format!("pwsh-test{sfx}")));
    }

    if cfg!(target_os = "windows") {
        let pdb = profile_dir.join("nvim.pdb");
        if pdb.exists() {
            let _ = fs::copy(&pdb, bin_dir.join("nvim.pdb"));
        }
        for dep_bin in ["lua51.dll", "win32yank.exe"] {
            let src = deps_prefix.join("bin").join(dep_bin);
            if src.exists() {
                let _ = fs::copy(&src, bin_dir.join(dep_bin));
            }
        }
    }

    let parser_src = deps_prefix.join("lib").join("nvim").join("parser");
    if parser_src.exists() {
        deps_build::copy_dir_all(&parser_src, &lib_nvim_dir.join("parser")).unwrap();
    }

    let nvim_bin = bin_dir.join(format!("nvim{sfx}"));
    let preload = root.join("src").join("gen").join("preload.lua");
    let runtime_dir = root.join("runtime");

    // 1. Generate runtime/syntax/vim/generated.vim
    let gen_syn_vim = gen_rt_dir.join("syntax").join("vim").join("generated.vim");
    let mut cmd = Command::new(&nvim_bin);
    cmd.current_dir(root)
        .env("VIMRUNTIME", &runtime_dir)
        .arg("-u")
        .arg("NONE")
        .arg("-l")
        .arg(&preload)
        .arg(root)
        .arg(root.join("src").join("gen").join("gen_vimvim.lua"))
        .arg(&gen_syn_vim)
        .arg(root.join("src").join("nvim").join("eval.lua"))
        .arg(root.join("src").join("nvim").join("options.lua"))
        .arg(root.join("src").join("nvim").join("auevents.lua"))
        .arg(root.join("src").join("nvim").join("ex_cmds.lua"))
        .arg(root.join("src").join("nvim").join("vvars.lua"));
    run_status(&mut cmd, "gen_vimvim.lua");

    // 2. Generate runtime/doc/tags
    let build_doc_dir = gen_rt_dir.join("doc");
    if build_doc_dir.exists() {
        let _ = fs::remove_dir_all(&build_doc_dir);
    }
    deps_build::copy_dir_all(&root.join("runtime").join("doc"), &build_doc_dir).unwrap();
    let helptags_lua = root.join("src").join("gen").join("gen_helptags.lua");
    let mut cmd = Command::new(&nvim_bin);
    cmd.current_dir(&gen_rt_dir)
        .env("VIMRUNTIME", &runtime_dir)
        .args(["-u", "NONE", "-l"])
        .arg(&helptags_lua)
        .args(["doc/tags", "doc", "++t"]);
    run_status(&mut cmd, "gen_helptags.lua doc/tags");

    // 3. Generate runtime/pack/dist/opt/*/doc/tags
    let opt_src_dir = root.join("runtime").join("pack").join("dist").join("opt");
    if opt_src_dir.exists() {
        for entry in fs::read_dir(&opt_src_dir).unwrap() {
            let entry = entry.unwrap();
            let pkg_path = entry.path();
            if !pkg_path.join("doc").is_dir() {
                continue;
            }
            let dst_pkg = gen_rt_dir
                .join("pack")
                .join("dist")
                .join("opt")
                .join(entry.file_name());
            deps_build::copy_dir_all(&pkg_path, &dst_pkg).unwrap();
            let mut cmd = Command::new(&nvim_bin);
            cmd.current_dir(&dst_pkg)
                .env("VIMRUNTIME", &runtime_dir)
                .args(["-u", "NONE", "-l"])
                .arg(&helptags_lua)
                .args(["doc/tags", "doc"]);
            run_status(
                &mut cmd,
                &format!("gen_helptags.lua {}", entry.file_name().to_string_lossy()),
            );
        }
    }

    // 4. Write target/nvim-build/test/config/paths.lua
    let test_config_dir = build_dir.join("test").join("config");
    fs::create_dir_all(&test_config_dir).unwrap();
    let norm = |p: &Path| p.to_string_lossy().replace('\\', "/");
    let luajit_bin = deps_prefix.join("bin").join(format!("luajit{sfx}"));
    let lua_prg = if luajit_bin.exists() {
        norm(&luajit_bin)
    } else {
        "luajit".to_string()
    };
    let vterm_file = norm(&build_dir.join("test").join("vterm_test_output"));
    let paths_lua = format!(
        "local M = {{}}\n\
         M.translations_enabled = false\n\
         M.is_asan = false\n\
         M.vterm_test_file = {:?}\n\
         M.test_build_dir = {:?}\n\
         M.test_source_path = {:?}\n\
         M.test_lua_prg = {:?}\n\
         M.test_luajit_prg = {:?}\n\
         return M\n",
        vterm_file,
        norm(build_dir),
        norm(root),
        lua_prg,
        lua_prg,
    );
    fs::write(test_config_dir.join("paths.lua"), paths_lua).unwrap();

    nvim_bin
}

fn install_prefix(root: &Path, profile_dir: &Path, build_dir: &Path, prefix: &Path) {
    stage_build_dir(root, profile_dir, build_dir);

    let deps_prefix = default_deps_prefix(root);
    let dst_bin = prefix.join("bin");
    let dst_lib = prefix.join("lib");
    let dst_share = prefix.join("share");
    let dst_rt = dst_share.join("nvim").join("runtime");
    fs::create_dir_all(&dst_bin).unwrap();
    fs::create_dir_all(&dst_lib).unwrap();
    fs::create_dir_all(&dst_rt).unwrap();

    let sfx = exe_suffix();
    for bin in ["nvim", "tee", "xxd"] {
        let src = build_dir.join("bin").join(format!("{bin}{sfx}"));
        if src.exists() {
            fs::copy(&src, dst_bin.join(format!("{bin}{sfx}"))).unwrap();
        }
    }
    if cfg!(target_os = "windows") {
        for f in ["nvim.pdb", "lua51.dll", "win32yank.exe"] {
            let src = build_dir.join("bin").join(f);
            if src.exists() {
                fs::copy(&src, dst_bin.join(f)).unwrap();
            }
        }
    }

    let parser_dir = build_dir.join("lib").join("nvim");
    if parser_dir.exists() {
        deps_build::copy_dir_all(&parser_dir, &dst_lib.join("nvim")).unwrap();
    }

    let man_dir = dst_share.join("man").join("man1");
    fs::create_dir_all(&man_dir).unwrap();
    fs::copy(
        root.join("src").join("man").join("nvim.1"),
        man_dir.join("nvim.1"),
    )
    .unwrap();

    let icon_dir = dst_share
        .join("icons")
        .join("hicolor")
        .join("128x128")
        .join("apps");
    fs::create_dir_all(&icon_dir).unwrap();
    fs::copy(
        root.join("runtime").join("nvim.png"),
        icon_dir.join("nvim.png"),
    )
    .unwrap();

    if !cfg!(target_os = "macos") {
        let app_dir = dst_share.join("applications");
        fs::create_dir_all(&app_dir).unwrap();
        fs::copy(
            root.join("runtime").join("org.neovim.nvim.desktop"),
            app_dir.join("org.neovim.nvim.desktop"),
        )
        .unwrap();
    }

    let src_rt = root.join("runtime");
    for entry in fs::read_dir(&src_rt).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if path.is_dir() {
            deps_build::copy_dir_all(&path, &dst_rt.join(&name)).unwrap();
        } else if name_str.ends_with(".vim")
            || name_str.ends_with(".lua")
            || name_str.ends_with(".ico")
        {
            fs::copy(&path, dst_rt.join(&name)).unwrap();
        }
    }

    let jit_dir = deps_prefix.join("share").join("luajit-2.1").join("jit");
    if jit_dir.exists() {
        deps_build::copy_dir_all(&jit_dir, &dst_rt.join("lua").join("jit")).unwrap();
    }

    // Overlay generated runtime files (doc/tags, syntax/vim/generated.vim, pack/dist/opt/*/doc/tags)
    deps_build::copy_dir_all(&build_dir.join("runtime"), &dst_rt).unwrap();
}

fn create_symlink_or_junction(src: &Path, dst: &Path) {
    if dst.exists() || dst.symlink_metadata().is_ok() {
        if dst.is_dir() {
            let _ = fs::remove_dir_all(dst);
        } else {
            let _ = fs::remove_file(dst);
        }
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(src, dst).expect("failed to create symlink");
    }
    #[cfg(windows)]
    {
        if src.is_dir() {
            let mut cmd = Command::new("cmd.exe");
            cmd.args(["/d", "/c", "mklink", "/J"]).arg(dst).arg(src);
            let out = cmd.output().expect("failed to run mklink /J");
            assert!(
                out.status.success(),
                "mklink /J failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        } else {
            fs::copy(src, dst).expect("failed to copy file into Xtest_xdg");
        }
    }
}

fn run_lua_tests(
    root: &Path,
    build_dir: &Path,
    nvim_bin: &Path,
    test_type: &str,
    extra_args: &[String],
) {
    let xdg_dir = build_dir.join("Xtest_xdg");
    let tmp_dir = build_dir.join("Xtest_tmpdir");
    if xdg_dir.exists() {
        let _ = fs::remove_dir_all(&xdg_dir);
    }
    if tmp_dir.exists() {
        let _ = fs::remove_dir_all(&tmp_dir);
    }
    fs::create_dir_all(&xdg_dir).unwrap();
    fs::create_dir_all(&tmp_dir).unwrap();

    create_symlink_or_junction(&root.join("runtime"), &xdg_dir.join("runtime"));
    create_symlink_or_junction(&root.join("src"), &xdg_dir.join("src"));
    create_symlink_or_junction(&root.join("test"), &xdg_dir.join("test"));
    create_symlink_or_junction(&root.join("README.md"), &xdg_dir.join("README.md"));

    let log_file = env::var("NVIM_LOG_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| build_dir.join("nvim.log"));

    let test_path = env::var("TEST_FILE").unwrap_or_else(|_| format!("test/{test_type}"));

    let norm_slashes = |p: &Path| p.to_string_lossy().replace('\\', "/");
    let mut cmd = Command::new(nvim_bin);
    cmd.current_dir(&xdg_dir)
        .env("NVIM_TEST", "1")
        .env("LC_ALL", "en_US.UTF-8")
        .env("NVIM_LOG_FILE", &log_file)
        .env("NVIM_PRG", norm_slashes(nvim_bin))
        .env("TMPDIR", &tmp_dir)
        .env("HISTFILE", "/dev/null")
        .env("SYSTEM_NAME", system_name())
        .env("NVIM_TEST_SOURCE_DIR", norm_slashes(root))
        .env("NVIM_TEST_BUILD_DIR", norm_slashes(build_dir))
        .env_remove("XDG_DATA_DIRS")
        .env_remove("NVIM")
        .env_remove("TMUX");
    if !cfg!(target_os = "windows") {
        cmd.env("SHELL", "sh");
    }

    cmd.arg("-l")
        .arg(root.join("test").join("runner.lua"))
        .arg(format!("-X{}", xdg_dir.display()))
        .arg("-v")
        .arg("--summary-file=-")
        .arg(format!(
            "--helper={}",
            root.join("test")
                .join(test_type)
                .join("preload.lua")
                .display()
        ))
        .arg(format!(
            "--lpath={}/?.lua",
            build_dir.to_string_lossy().replace('\\', "/")
        ));

    if let Ok(tag) = env::var("TEST_TAG")
        && !tag.is_empty()
    {
        cmd.args(["--tags", &tag]);
    }
    if let Ok(filter) = env::var("TEST_FILTER")
        && !filter.is_empty()
    {
        cmd.args(["--filter", &filter]);
    }
    if let Ok(filter_out) = env::var("TEST_FILTER_OUT")
        && !filter_out.is_empty()
    {
        cmd.args(["--filter-out", &filter_out]);
    }
    for arg in extra_args {
        cmd.arg(arg);
    }
    cmd.arg(&test_path);

    let status = cmd.status().expect("failed to execute test runner");
    let _ = fs::remove_dir_all(&xdg_dir);
    let _ = fs::remove_dir_all(&tmp_dir);
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn default_install_prefix(target_dir: &Path) -> PathBuf {
    if cfg!(target_os = "windows") {
        if let Ok(profile) = env::var("USERPROFILE") {
            return PathBuf::from(profile)
                .join(".local")
                .join("bin")
                .join("nvim-win64");
        }
    } else if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home).join(".local");
    }
    target_dir.join("release").join("install")
}

fn print_help() {
    eprintln!(
        "Usage: cargo xtask <subcommand> [options]\n\n\
         Subcommands:\n\
         \x20 deps [--force]              Download and compile bundled C dependencies (.deps)\n\
         \x20 build [--release]           Compile binaries and stage runtime in target/nvim-build/\n\
         \x20 runtime [--release]         Generate runtime files (tags, generated.vim) in target/nvim-build/\n\
         \x20 install [--prefix <dir>]    Build release and install Neovim (default: ~/.local/bin/nvim-win64 on Windows, ~/.local on Unix)\n\
         \x20 dist [--out-dir <dir>]      Build release archive in target/release/ (e.g. target/release/nvim-win64.zip)\n\
         \x20 doc / doc-vim / doc-eval    Regenerate documentation files\n\
         \x20 lint                        Run all linters (lintrust, lintlua, linterrcodes, lintdoc)\n\
         \x20 lintrust / formatrust       Run cargo clippy / cargo fmt\n\
         \x20 lintlua / formatlua         Run stylua check / format\n\
         \x20 lintdoc / lintdocurls       Run documentation linter\n\
         \x20 linterrcodes / lintcommit   Run error code / commit message linter\n\
         \x20 test                        Run testrust, unittest, and functionaltest\n\
         \x20 testrust                    Run Rust unit tests (cargo test)\n\
         \x20 unittest [args...]          Run LuaJIT FFI unit tests\n\
         \x20 functionaltest [args...]    Run functional test suite\n\
         \x20 oldtest [args...]           Run legacy Vim test suite\n\
         \x20 benchmark [args...]         Run benchmark suite\n\
         \x20 clean / distclean           Remove target/ build artifacts (distclean also removes .deps)"
    );
}

fn main() -> ExitCode {
    let root = repo_root();
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(subcmd) = args.first().map(String::as_str) else {
        print_help();
        return ExitCode::SUCCESS;
    };

    let release = args.iter().any(|a| a == "--release");
    let debug = args.iter().any(|a| a == "--debug");
    let force = args.iter().any(|a| a == "--force");
    let prefer_lua = env::var("PREFER_LUA").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("on"))
        || args.iter().any(|a| a == "--prefer-lua");

    let target_dir = cargo_target_dir(&root);
    let mut build_dir = target_dir.join("nvim-build");
    let mut prefix_dir = default_install_prefix(&target_dir);
    let mut out_dir = target_dir.join("release");
    let mut extra_args = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--release" | "--debug" | "--force" | "--prefer-lua" => {}
            "--build-dir" if i + 1 < args.len() => {
                build_dir = PathBuf::from(&args[i + 1]);
                i += 1;
            }
            "--prefix" if i + 1 < args.len() => {
                prefix_dir = PathBuf::from(&args[i + 1]);
                i += 1;
            }
            "--out-dir" if i + 1 < args.len() => {
                out_dir = PathBuf::from(&args[i + 1]);
                i += 1;
            }
            other => extra_args.push(other.to_string()),
        }
        i += 1;
    }

    match subcmd {
        "-h" | "--help" | "help" => {
            print_help();
        }
        "deps" => {
            let deps_prefix = default_deps_prefix(&root);
            deps_build::ensure_deps(&root, &deps_prefix, prefer_lua, force);
        }
        "runtime" => {
            let profile = if release { "release" } else { "debug" };
            let profile_dir = target_dir.join(profile);
            if !profile_dir.join(format!("nvim{}", exe_suffix())).exists() {
                build_cargo_bins(&root, release, prefer_lua);
            }
            stage_build_dir(&root, &profile_dir, &build_dir);
        }
        "build" => {
            let profile_dir = build_cargo_bins(&root, release, prefer_lua);
            stage_build_dir(&root, &profile_dir, &build_dir);
        }
        "install" => {
            let use_release = !debug;
            let profile_dir = build_cargo_bins(&root, use_release, prefer_lua);
            install_prefix(&root, &profile_dir, &build_dir, &prefix_dir);
            eprintln!("[install] Installed Neovim to {}", prefix_dir.display());
        }
        "dist" => {
            let use_release = !debug;
            let profile_dir = build_cargo_bins(&root, use_release, prefer_lua);
            let pkg_name = if cfg!(target_os = "windows") {
                if cfg!(target_arch = "aarch64") {
                    "nvim-win-arm64"
                } else {
                    "nvim-win64"
                }
            } else if cfg!(target_os = "macos") {
                if cfg!(target_arch = "aarch64") {
                    "nvim-macos-arm64"
                } else {
                    "nvim-macos-x86_64"
                }
            } else if cfg!(target_arch = "aarch64") {
                "nvim-linux-arm64"
            } else {
                "nvim-linux-x86_64"
            };

            fs::create_dir_all(&out_dir).unwrap();
            let stage_root = out_dir.join(pkg_name);
            if stage_root.exists() {
                fs::remove_dir_all(&stage_root).unwrap();
            }
            let tmp_build = out_dir.join("_stage_build");
            install_prefix(&root, &profile_dir, &tmp_build, &stage_root);
            let _ = fs::remove_dir_all(&tmp_build);

            if cfg!(target_os = "windows") {
                let archive = out_dir.join(format!("{pkg_name}.zip"));
                if archive.exists() {
                    let _ = fs::remove_file(&archive);
                }
                let mut cmd = Command::new("tar");
                cmd.current_dir(&out_dir)
                    .args(["-a", "-cf"])
                    .arg(&archive)
                    .arg(pkg_name);
                run_status(&mut cmd, "tar zip archive");
                eprintln!("[dist] Packaged {}", archive.display());
            } else {
                let archive = out_dir.join(format!("{pkg_name}.tar.gz"));
                if archive.exists() {
                    let _ = fs::remove_file(&archive);
                }
                let mut cmd = Command::new("tar");
                cmd.current_dir(&out_dir)
                    .args(["-czf"])
                    .arg(&archive)
                    .arg(pkg_name);
                run_status(&mut cmd, "tar.gz archive");
                eprintln!("[dist] Packaged {}", archive.display());
            }
        }
        "doc-vim" | "doc-eval" | "doc" | "lintdoc" | "lintdocurls" => {
            let profile_dir = build_cargo_bins(&root, release, prefer_lua);
            let nvim_bin = stage_build_dir(&root, &profile_dir, &build_dir);
            let preload = root.join("src").join("gen").join("preload.lua");
            let runtime_dir = root.join("runtime");

            if subcmd == "doc-vim" || subcmd == "doc" {
                let mut cmd = Command::new(&nvim_bin);
                cmd.current_dir(&root)
                    .env("VIMRUNTIME", &runtime_dir)
                    .args(["-u", "NONE", "-l"])
                    .arg(&preload)
                    .arg(&root)
                    .arg(root.join("src").join("gen").join("gen_vimdoc.lua"));
                run_status(&mut cmd, "gen_vimdoc.lua");
            }
            if subcmd == "doc-eval" || subcmd == "doc" {
                let mut cmd = Command::new(&nvim_bin);
                cmd.current_dir(&root)
                    .env("VIMRUNTIME", &runtime_dir)
                    .args(["-u", "NONE", "-l"])
                    .arg(&preload)
                    .arg(&root)
                    .arg(root.join("src").join("gen").join("gen_eval_files.lua"))
                    .arg(build_dir.join("runtime").join("doc").join("tags"));
                run_status(&mut cmd, "gen_eval_files.lua");
            }
            if subcmd == "lintdoc" || subcmd == "lintdocurls" {
                let mut cmd = Command::new(&nvim_bin);
                cmd.current_dir(&root)
                    .env("VIMRUNTIME", &runtime_dir)
                    .args(["-u", "NONE", "-l"])
                    .arg(&preload)
                    .arg(&root)
                    .arg(root.join("scripts").join("lintdoc.lua"));
                if subcmd == "lintdocurls" {
                    cmd.arg("true");
                }
                run_status(&mut cmd, subcmd);
            }
        }
        "lintrust" => {
            let deps_prefix = default_deps_prefix(&root);
            deps_build::ensure_deps(&root, &deps_prefix, prefer_lua, false);
            let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
            let mut cmd = Command::new(cargo);
            cmd.current_dir(&root).args([
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]);
            if prefer_lua {
                cmd.args(["--features", "vendor-bit"]);
            }
            run_status(&mut cmd, "cargo clippy");
        }
        "formatrust" => {
            let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
            let mut cmd = Command::new(cargo);
            cmd.current_dir(&root).arg("fmt");
            for a in &extra_args {
                cmd.arg(a);
            }
            run_status(&mut cmd, "cargo fmt");
        }
        "lintlua" | "formatlua" => {
            if Command::new("stylua").arg("--version").output().is_err() {
                if env::var("CI_LINT").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("on")) {
                    panic!("stylua not found in PATH");
                }
                eprintln!("[{subcmd}] stylua not found in PATH, skipping");
                return ExitCode::SUCCESS;
            }
            let check = subcmd == "lintlua";
            let mut cmd1 = Command::new("stylua");
            cmd1.current_dir(&root).arg("--respect-ignores");
            if check {
                cmd1.args(["--color=always", "--check"]);
            }
            cmd1.args(["runtime", "scripts", "src", "test", "contrib"]);
            run_status(&mut cmd1, "stylua");

            let mut cmd2 = Command::new("stylua");
            cmd2.current_dir(&root)
                .args(["--config-path", ".stylua2.toml"]);
            if check {
                cmd2.args(["--color=always", "--check"]);
            }
            cmd2.args([
                "test/functional/ui/decorations_spec.lua",
                "test/functional/ui/float_spec.lua",
                "test/functional/ui/multigrid_spec.lua",
            ]);
            run_status(&mut cmd2, "stylua (.stylua2.toml)");
        }
        "linterrcodes" | "lintcommit" => {
            let profile_dir = build_cargo_bins(&root, release, prefer_lua);
            let nvim_bin = stage_build_dir(&root, &profile_dir, &build_dir);
            let mut cmd = Command::new(&nvim_bin);
            cmd.current_dir(&root)
                .env("VIMRUNTIME", root.join("runtime"))
                .arg("--clean")
                .arg("-l");
            if subcmd == "linterrcodes" {
                cmd.arg(root.join("scripts").join("linterrcodes.lua"));
            } else {
                cmd.arg(root.join("scripts").join("lintcommit.lua"))
                    .arg("main");
            }
            run_status(&mut cmd, subcmd);
        }
        "lint" => {
            let exe = env::current_exe().unwrap();
            for step in ["lintrust", "lintlua", "linterrcodes", "lintdoc"] {
                let mut cmd = Command::new(&exe);
                cmd.current_dir(&root).arg(step);
                run_status(&mut cmd, step);
            }
        }
        "format" => {
            let exe = env::current_exe().unwrap();
            for step in ["formatrust", "formatlua"] {
                let mut cmd = Command::new(&exe);
                cmd.current_dir(&root).arg(step);
                run_status(&mut cmd, step);
            }
        }
        "testrust" => {
            let deps_prefix = default_deps_prefix(&root);
            deps_build::ensure_deps(&root, &deps_prefix, prefer_lua, false);
            let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
            let mut cmd = Command::new(cargo);
            cmd.current_dir(&root).arg("test");
            if prefer_lua {
                cmd.args(["--features", "vendor-bit"]);
            }
            for a in &extra_args {
                cmd.arg(a);
            }
            run_status(&mut cmd, "cargo test");
        }
        "unittest" | "functionaltest" | "benchmark" => {
            let profile_dir = build_cargo_bins(&root, release, prefer_lua);
            let nvim_bin = stage_build_dir(&root, &profile_dir, &build_dir);
            let test_type = match subcmd {
                "unittest" => "unit",
                "functionaltest" => "functional",
                _ => "benchmark",
            };
            run_lua_tests(&root, &build_dir, &nvim_bin, test_type, &extra_args);
        }
        "oldtest" => {
            let profile_dir = build_cargo_bins(&root, release, prefer_lua);
            let nvim_bin = stage_build_dir(&root, &profile_dir, &build_dir);
            let mut cmd = Command::new(&nvim_bin);
            cmd.current_dir(&root)
                .args(["--clean", "-l"])
                .arg(root.join("test").join("old").join("runner.lua"));
            for a in &extra_args {
                cmd.arg(a);
            }
            run_status(&mut cmd, "oldtest");
        }
        "test" => {
            let exe = env::current_exe().unwrap();
            for step in ["testrust", "unittest", "functionaltest"] {
                let mut cmd = Command::new(&exe);
                cmd.current_dir(&root).arg(step);
                if release {
                    cmd.arg("--release");
                }
                run_status(&mut cmd, step);
            }
        }
        "clean" | "distclean" => {
            let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
            let mut cmd = Command::new(cargo);
            cmd.current_dir(&root).arg("clean");
            let _ = cmd.status();
            for dir in ["build", "build-release"] {
                let p = root.join(dir);
                if p.exists() {
                    let _ = fs::remove_dir_all(&p);
                }
            }
            if subcmd == "distclean" {
                let deps = root.join(".deps");
                if deps.exists() {
                    let _ = fs::remove_dir_all(&deps);
                }
            }
        }
        other => {
            eprintln!("Unknown xtask subcommand: {other}");
            print_help();
            return ExitCode::FAILURE;
        }
    }

    ExitCode::SUCCESS
}
