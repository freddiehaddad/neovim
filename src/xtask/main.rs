use std::collections::{BTreeMap, BTreeSet, HashMap};
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
    let paths_lua = format!(
        "local M = {{}}\n\
         M.translations_enabled = false\n\
         M.is_asan = false\n\
         M.test_build_dir = {:?}\n\
         M.test_source_path = {:?}\n\
         M.test_lua_prg = {:?}\n\
         M.test_luajit_prg = {:?}\n\
         return M\n",
        norm(build_dir),
        norm(root),
        lua_prg,
        lua_prg,
    );
    fs::write(test_config_dir.join("paths.lua"), paths_lua).unwrap();

    nvim_bin
}

fn unescape_po_string(raw: &[u8]) -> Vec<u8> {
    let mut s = raw;
    while s.first().is_some_and(u8::is_ascii_whitespace) {
        s = &s[1..];
    }
    while s.last().is_some_and(u8::is_ascii_whitespace) {
        s = &s[..s.len() - 1];
    }
    if s.len() >= 2 && s[0] == b'"' && s[s.len() - 1] == b'"' {
        s = &s[1..s.len() - 1];
    }
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'\\' && i + 1 < s.len() {
            i += 1;
            match s[i] {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'r' => out.push(b'\r'),
                b'"' => out.push(b'"'),
                b'\\' => out.push(b'\\'),
                b'0' => out.push(0),
                other => out.push(other),
            }
        } else {
            out.push(s[i]);
        }
        i += 1;
    }
    out
}

fn compile_po_to_mo_rust(po_path: &Path, mo_path: &Path) {
    let data = fs::read(po_path).expect("failed to read .po file");
    let mut entries: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();

    #[derive(PartialEq, Eq)]
    enum State {
        None,
        Msgid,
        MsgidPlural,
        Msgstr(usize),
    }

    let mut state = State::None;
    let mut fuzzy = false;
    let mut msgid = Vec::new();
    let mut msgid_plural: Option<Vec<u8>> = None;
    let mut msgstrs: Vec<Vec<u8>> = Vec::new();

    let flush = |entries: &mut BTreeMap<Vec<u8>, Vec<u8>>,
                 fuzzy: &mut bool,
                 msgid: &mut Vec<u8>,
                 msgid_plural: &mut Option<Vec<u8>>,
                 msgstrs: &mut Vec<Vec<u8>>| {
        if !*fuzzy && !msgstrs.is_empty() && msgstrs.iter().any(|s| !s.is_empty()) {
            let mut key = std::mem::take(msgid);
            if let Some(plural) = msgid_plural.take() {
                key.push(0);
                key.extend_from_slice(&plural);
            }
            let mut val = Vec::new();
            for (idx, part) in msgstrs.iter().enumerate() {
                if idx > 0 {
                    val.push(0);
                }
                val.extend_from_slice(part);
            }
            entries.insert(key, val);
        }
        *fuzzy = false;
        msgid.clear();
        *msgid_plural = None;
        msgstrs.clear();
    };

    for raw_line in data.split(|&b| b == b'\n') {
        let mut line = raw_line;
        while line.first().is_some_and(u8::is_ascii_whitespace) {
            line = &line[1..];
        }
        while line.last().is_some_and(u8::is_ascii_whitespace) {
            line = &line[..line.len() - 1];
        }
        if line.is_empty() {
            flush(
                &mut entries,
                &mut fuzzy,
                &mut msgid,
                &mut msgid_plural,
                &mut msgstrs,
            );
            state = State::None;
            continue;
        }
        if line.starts_with(b"#") {
            if line.starts_with(b"#,") && line.windows(5).any(|w| w == b"fuzzy") {
                fuzzy = true;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix(b"msgid_plural ") {
            state = State::MsgidPlural;
            msgid_plural = Some(unescape_po_string(rest));
        } else if let Some(rest) = line.strip_prefix(b"msgid ") {
            if matches!(state, State::Msgstr(_)) {
                flush(
                    &mut entries,
                    &mut fuzzy,
                    &mut msgid,
                    &mut msgid_plural,
                    &mut msgstrs,
                );
            }
            state = State::Msgid;
            msgid = unescape_po_string(rest);
        } else if let Some(rest) = line.strip_prefix(b"msgstr ") {
            state = State::Msgstr(0);
            msgstrs.clear();
            msgstrs.push(unescape_po_string(rest));
        } else if let Some(rest) = line.strip_prefix(b"msgstr[") {
            if let Some(bracket) = rest.iter().position(|&b| b == b']') {
                let idx = std::str::from_utf8(&rest[..bracket])
                    .ok()
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                let val_part = &rest[bracket + 1..];
                if msgstrs.len() <= idx {
                    msgstrs.resize_with(idx + 1, Vec::new);
                }
                msgstrs[idx] = unescape_po_string(val_part);
                state = State::Msgstr(idx);
            }
        } else if line.starts_with(b"\"") {
            let chunk = unescape_po_string(line);
            match state {
                State::Msgid => msgid.extend_from_slice(&chunk),
                State::MsgidPlural => {
                    if let Some(p) = &mut msgid_plural {
                        p.extend_from_slice(&chunk);
                    }
                }
                State::Msgstr(idx) => {
                    if let Some(s) = msgstrs.get_mut(idx) {
                        s.extend_from_slice(&chunk);
                    }
                }
                State::None => {}
            }
        }
    }
    flush(
        &mut entries,
        &mut fuzzy,
        &mut msgid,
        &mut msgid_plural,
        &mut msgstrs,
    );

    let n = entries.len() as u32;
    let orig_tab_offset = 28u32;
    let trans_tab_offset = 28 + 8 * n;
    let hash_tab_offset = 28 + 16 * n;
    let mut cur_offset = hash_tab_offset;

    let mut orig_tab = Vec::with_capacity((n * 8) as usize);
    let mut trans_tab = Vec::with_capacity((n * 8) as usize);
    let mut pool = Vec::new();

    for key in entries.keys() {
        orig_tab.extend_from_slice(&(key.len() as u32).to_le_bytes());
        orig_tab.extend_from_slice(&cur_offset.to_le_bytes());
        pool.extend_from_slice(key);
        pool.push(0);
        cur_offset += key.len() as u32 + 1;
    }
    for val in entries.values() {
        trans_tab.extend_from_slice(&(val.len() as u32).to_le_bytes());
        trans_tab.extend_from_slice(&cur_offset.to_le_bytes());
        pool.extend_from_slice(val);
        pool.push(0);
        cur_offset += val.len() as u32 + 1;
    }

    let mut out = Vec::with_capacity(cur_offset as usize);
    out.extend_from_slice(&0x9504_12de_u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&orig_tab_offset.to_le_bytes());
    out.extend_from_slice(&trans_tab_offset.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&hash_tab_offset.to_le_bytes());
    out.extend_from_slice(&orig_tab);
    out.extend_from_slice(&trans_tab);
    out.extend_from_slice(&pool);

    fs::write(mo_path, out).expect("failed to write .mo file");
}

fn install_translations(root: &Path, dst_share: &Path) {
    let po_dir = root.join("src").join("nvim").join("po");
    if !po_dir.exists() {
        return;
    }
    let has_msgfmt = Command::new("msgfmt").arg("--version").output().is_ok();
    for entry in fs::read_dir(&po_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("po") {
            continue;
        }
        let Some(lang) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if matches!(lang, "az" | "sr") {
            continue;
        }
        let lc_dir = dst_share.join("locale").join(lang).join("LC_MESSAGES");
        fs::create_dir_all(&lc_dir).unwrap();
        let mo_path = lc_dir.join("nvim.mo");
        if has_msgfmt {
            let status = Command::new("msgfmt")
                .env("OLD_PO_FILE_INPUT", "yes")
                .arg("-o")
                .arg(&mo_path)
                .arg(&path)
                .status();
            if status.is_ok_and(|s| s.success()) {
                continue;
            }
        }
        compile_po_to_mo_rust(&path, &mo_path);
    }
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

    // Compile and install gettext locale translations (share/locale/<lang>/LC_MESSAGES/nvim.mo)
    install_translations(root, &dst_share);
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
    let tmp_dir = root.join("Xtest_tmpdir");
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
        .env("TMPDIR", norm_slashes(&tmp_dir))
        .env("HISTFILE", "/dev/null")
        .env("SYSTEM_NAME", system_name())
        .env("VIMRUNTIME", norm_slashes(&root.join("runtime")))
        .env("NVIM_TEST_SOURCE_DIR", norm_slashes(root))
        .env("NVIM_TEST_BUILD_DIR", norm_slashes(build_dir))
        .env_remove("XDG_DATA_DIRS")
        .env_remove("NVIM")
        .env_remove("TMUX")
        .env_remove("NoDefaultCurrentDirectoryInExePath");
    if !cfg!(target_os = "windows") {
        cmd.env("SHELL", "sh");
    } else if let Ok(path) = env::var("PATH") {
        let git_usr_bin = Path::new(r"C:\Program Files\Git\usr\bin");
        if git_usr_bin.is_dir() && !path.to_ascii_lowercase().contains(r"git\usr\bin") {
            cmd.env(
                "PATH",
                format!(r"C:\Windows\System32;{};{path}", git_usr_bin.display()),
            );
        }
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
    let mut has_positional = false;
    for arg in extra_args {
        if !arg.starts_with('-') {
            has_positional = true;
        }
        cmd.arg(arg);
    }
    if !has_positional {
        cmd.arg(&test_path);
    }

    let status = cmd.status().expect("failed to execute test runner");
    let _ = fs::remove_dir_all(&xdg_dir);
    let _ = fs::remove_dir_all(&tmp_dir);
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn extract_errcodes(line: &str) -> Vec<String> {
    let code_part = line.split_once("//").map_or(line, |(before, _)| before);
    let bytes = code_part.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'E' {
            let prev_is_word =
                i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            let mut d = i + 1;
            while d < bytes.len() && d < i + 5 && bytes[d].is_ascii_digit() {
                d += 1;
            }
            let num_digits = d - (i + 1);
            let next_is_digit = d < bytes.len() && bytes[d].is_ascii_digit();
            if !prev_is_word && (num_digits == 3 || num_digits == 4) && !next_is_digit {
                out.push(code_part[i..d].to_string());
                i = d;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn run_linterrcodes(root: &Path) {
    let dup_allowed: HashMap<&str, usize> = HashMap::from([
        ("E109", 2),
        ("E110", 2),
        ("E112", 3),
        ("E114", 2),
        ("E115", 2),
        ("E1159", 2),
        ("E116", 2),
        ("E121", 2),
        ("E1239", 2),
        ("E136", 5),
        ("E1502", 4),
        ("E1510", 2),
        ("E151", 2),
        ("E155", 3),
        ("E170", 2),
        ("E173", 2),
        ("E176", 2),
        ("E177", 2),
        ("E178", 2),
        ("E180", 2),
        ("E212", 2),
        ("E216", 2),
        ("E298", 3),
        ("E303", 3),
        ("E312", 2),
        ("E317", 4),
        ("E319", 2),
        ("E367", 3),
        ("E423", 3),
        ("E474", 60),
        ("E475", 6),
        ("E482", 3),
        ("E484", 3),
        ("E488", 2),
        ("E5009", 3),
        ("E502", 2),
        ("E503", 3),
        ("E504", 2),
        ("E505", 2),
        ("E509", 2),
        ("E5101", 2),
        ("E5102", 2),
        ("E5108", 6),
        ("E5111", 2),
        ("E513", 2),
        ("E521", 2),
        ("E5210", 2),
        ("E546", 2),
        ("E575", 27),
        ("E576", 9),
        ("E588", 2),
        ("E678", 2),
        ("E684", 2),
        ("E685", 5),
        ("E697", 2),
        ("E703", 2),
        ("E716", 2),
        ("E723", 2),
        ("E724", 3),
        ("E728", 2),
        ("E741", 2),
        ("E742", 2),
        ("E745", 2),
        ("E798", 2),
        ("E805", 2),
        ("E856", 3),
        ("E867", 2),
        ("E886", 10),
        ("E900", 3),
        ("E903", 2),
        ("E905", 2),
        ("E906", 2),
        ("E948", 2),
        ("E970", 2),
        ("E974", 2),
        ("E979", 2),
        ("E996", 5),
    ]);

    let help_out = Command::new("git")
        .current_dir(root)
        .args(["grep", "-hE", r"\*E[0-9]{3,4}\*", "--", "runtime/doc/*.txt"])
        .output()
        .expect("failed to run git grep on runtime/doc/*.txt");
    assert!(
        help_out.status.success(),
        "git grep runtime/doc/*.txt failed"
    );
    let mut help_codes = BTreeSet::new();
    for line in String::from_utf8_lossy(&help_out.stdout).lines() {
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'E' {
                let mut d = i + 1;
                while d < bytes.len() && d < i + 5 && bytes[d].is_ascii_digit() {
                    d += 1;
                }
                let num_digits = d - (i + 1);
                let next_is_digit = d < bytes.len() && bytes[d].is_ascii_digit();
                if (num_digits == 3 || num_digits == 4) && !next_is_digit {
                    help_codes.insert(line[i..d].to_string());
                    i = d;
                    continue;
                }
            }
            i += 1;
        }
    }

    let src_out = Command::new("git")
        .current_dir(root)
        .args(["grep", "-nE", "E[0-9]{3,4}", "--", "src/nvim/*.rs"])
        .output()
        .expect("failed to run git grep on src/nvim/*.rs");
    assert!(src_out.status.success(), "git grep src/nvim/*.rs failed");
    let mut src_codes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in String::from_utf8_lossy(&src_out.stdout).lines() {
        let content = line.splitn(3, ':').nth(2).unwrap_or(line);
        for code in extract_errcodes(content) {
            src_codes.entry(code).or_default().push(line.to_string());
        }
    }

    let code_num = |c: &str| c[1..].parse::<u32>().unwrap_or(0);
    let mut missing: Vec<String> = src_codes
        .keys()
        .filter(|c| !help_codes.contains(*c))
        .cloned()
        .collect();
    missing.sort_by_key(|c| code_num(c));

    let mut dup_codes: Vec<String> = src_codes
        .iter()
        .filter_map(|(code, locs)| {
            if let Some(&allowed) = dup_allowed.get(code.as_str()) {
                (locs.len() != allowed).then(|| code.clone())
            } else {
                (locs.len() > 1).then(|| code.clone())
            }
        })
        .collect();
    dup_codes.sort_by_key(|c| code_num(c));

    if !missing.is_empty() {
        println!("Error codes missing from help docs:");
        for code in &missing {
            println!("  {code}");
        }
        println!();
    }

    if !dup_codes.is_empty() {
        println!("Error codes used in more than one place:");
        for code in &dup_codes {
            let locs = &src_codes[code];
            println!("  {code} ({} occurrences):", locs.len());
            for loc in locs {
                println!("    {loc}");
            }
        }
        println!();
    }

    let max_code = src_codes.keys().map(|c| code_num(c)).max().unwrap_or(0);
    println!(
        "errcodes={} dup-codes={} missing-help={} highest=E{max_code}",
        src_codes.len(),
        dup_codes.len(),
        missing.len()
    );
    if !missing.is_empty() || !dup_codes.is_empty() {
        std::process::exit(1);
    }
}

fn validate_commit_msg(commit_message: &str) -> Option<String> {
    let commit_split: Vec<&str> = commit_message.split(':').collect();
    if commit_split.first() == Some(&"vim-patch") {
        return None;
    }
    if let Some(rest) = commit_message.strip_prefix("NVIM v") {
        let mut parts = rest.split('.');
        if parts
            .next()
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            && parts
                .next()
                .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            && parts
                .next()
                .is_some_and(|s| s.bytes().take_while(|b| b.is_ascii_digit()).count() > 0)
        {
            return None;
        }
    }

    if commit_message.len() > 80 {
        return Some(
            "Commit message is too long, a maximum of 80 characters is allowed.".to_string(),
        );
    }

    let mut before_colon = commit_split[0];
    let mut after_idx = 1usize;
    if before_colon.contains('(') && !before_colon.contains(')') {
        while after_idx < commit_split.len() {
            after_idx += 1;
            if commit_split[after_idx - 1].contains(')') {
                break;
            }
        }
    }
    if after_idx >= commit_split.len() {
        return Some("Commit message does not include colons.".to_string());
    }
    let after_colon = commit_split[after_idx..].join(":");

    if let Some(stripped) = before_colon.strip_suffix('!') {
        before_colon = stripped;
    }

    let allowed_types = [
        "build",
        "ci",
        "docs",
        "feat",
        "fix",
        "perf",
        "refactor",
        "revert",
        "sync",
        "test",
        "vim-patch",
    ];
    let commit_type = before_colon.split('(').next().unwrap_or("");
    if !allowed_types.contains(&commit_type) {
        return Some(format!(
            "Invalid commit type \"{commit_type}\". Allowed types are: {allowed_types:?}."
        ));
    }

    if let Some(open_idx) = commit_message.find('(')
        && before_colon.contains('(')
    {
        let close_idx = commit_message[open_idx + 1..]
            .find(')')
            .map(|i| open_idx + 1 + i)
            .unwrap_or(commit_message.len());
        let scope = commit_message[open_idx + 1..close_idx].trim();
        if scope.is_empty() {
            return Some("Scope can't be empty".to_string());
        }
        if scope.starts_with("nvim_") {
            return Some("Scope should be \"api\" instead of \"nvim_...\"".to_string());
        }
        let alt = match scope {
            "filetype.vim" | "filetype.lua" => Some("filetype"),
            "tree-sitter" | "ts" => Some("treesitter"),
            "hl" => Some("highlight"),
            _ => None,
        };
        if let Some(replacement) = alt {
            return Some(format!(
                "Scope should be \"{replacement}\" instead of \"{scope}\""
            ));
        }
    }

    if after_colon.ends_with('.') {
        return Some("Description ends with a period (\".\").".to_string());
    }
    if !after_colon.starts_with(' ') {
        return Some("There should be a whitespace after the colon.".to_string());
    }
    if after_colon.starts_with("  ") {
        return Some("There should only be one whitespace after the colon.".to_string());
    }
    let trimmed_start = after_colon.trim_start_matches(' ');
    let bytes = trimmed_start.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_uppercase() && bytes[1].is_ascii_lowercase() {
        return Some("Description first word should not be Capitalized.".to_string());
    }
    if after_colon.trim().is_empty() {
        return Some("Description shouldn't be empty.".to_string());
    }
    None
}

fn run_lintcommit(root: &Path) {
    let test_cases = [
        ("ci: normal message", true),
        ("build: normal message", true),
        ("build: version bump", true),
        ("docs: normal message", true),
        ("feat: normal message", true),
        ("fix: normal message", true),
        ("perf: normal message", true),
        ("refactor: normal message", true),
        ("revert: normal message", true),
        ("test: normal message", true),
        ("ci(window): message with scope", true),
        ("ci!: message with breaking change", true),
        ("ci(tui)!: message with scope and breaking change", true),
        ("NVIM v0.12.0", true),
        (
            "revert: \"ci: use continue-on-error instead of \"|| true\"\"",
            true,
        ),
        ("fixup", false),
        ("fixup: commit message", false),
        ("fixup! commit message", false),
        (":no type before colon 1", false),
        (" :no type before colon 2", false),
        ("  :no type before colon 3", false),
        ("ci(empty description):", false),
        ("ci(only whitespace as description): ", false),
        ("docs(multiple whitespaces as description):   ", false),
        ("ci no colon after type", false),
        ("test:  extra space after colon", false),
        ("ci:\ttab after colon", false),
        ("ci:no space after colon", false),
        ("ci :extra space before colon", false),
        ("refactor(): empty scope", false),
        ("ci( ): whitespace as scope", false),
        ("ci: period at end of sentence.", false),
        ("ci: period: at end of sentence.", false),
        ("ci: Capitalized first word", false),
        ("ci: UPPER_CASE First Word", true),
        ("unknown: using unknown type", false),
        ("feat: foo:bar", true),
        ("feat(something): foo:bar", true),
        ("feat(:grep): read from pipe", true),
        ("feat(:grep/:make): read from pipe", true),
        ("feat(:grep)", false),
        ("feat(:grep", false),
        (
            "ci: you're saying this commit message just goes on and on and on and on and on and on for way too long?",
            false,
        ),
    ];
    for (msg, expected) in test_cases {
        let actual = validate_commit_msg(msg).is_none();
        assert_eq!(
            actual, expected,
            "lintcommit self-test failed for {msg:?}: expected {expected}, got {actual}"
        );
    }

    let git_out = |args: &[&str]| -> Option<String> {
        let out = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };

    let range = if git_out(&["rev-parse", "--verify", "--quiet", "HEAD^2"]).is_some() {
        "HEAD^1..HEAD^2".to_string()
    } else {
        let ancestor = git_out(&["merge-base", "origin/master", "HEAD"])
            .or_else(|| git_out(&["merge-base", "upstream/master", "HEAD"]))
            .or_else(|| git_out(&["merge-base", "master", "HEAD"]))
            .expect("git merge-base failed");
        format!("{ancestor}..HEAD")
    };
    let commits_str = git_out(&["rev-list", "--no-merges", &range]).expect("git rev-list failed");

    let mut failed = 0usize;
    for commit_id in commits_str.split_whitespace() {
        if let Some(msg) = git_out(&["show", "-s", "--format=%s", commit_id])
            && let Some(err) = validate_commit_msg(&msg)
        {
            failed += 1;
            eprintln!("Invalid commit message: \"{msg}\"\n    Commit: {commit_id}\n    {err}\n");
        }
    }
    if failed > 0 {
        eprintln!(
            "See also:\n    https://github.com/freddiehaddad/neovim/blob/master/CONTRIBUTING.md#commit-messages\n"
        );
        std::process::exit(1);
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
         \x20 test                        Run unittest and functionaltest\n\
         \x20 unittest / testrust [args]  Run Rust unit tests (cargo test)\n\
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

                let wix_bin = if Command::new("wix")
                    .env("DOTNET_ROLL_FORWARD", "Major")
                    .arg("--version")
                    .output()
                    .is_ok_and(|o| o.status.success())
                {
                    Some(PathBuf::from("wix"))
                } else {
                    env::var("USERPROFILE").ok().and_then(|p| {
                        let candidate = PathBuf::from(p)
                            .join(".dotnet")
                            .join("tools")
                            .join("wix.exe");
                        candidate.exists().then_some(candidate)
                    })
                };
                if let Some(wix) = wix_bin {
                    let msi_path = out_dir.join(format!("{pkg_name}.msi"));
                    if msi_path.exists() {
                        let _ = fs::remove_file(&msi_path);
                    }
                    let ver = Command::new(stage_root.join("bin").join("nvim.exe"))
                        .arg("--version")
                        .output()
                        .ok()
                        .and_then(|o| {
                            let s = String::from_utf8_lossy(&o.stdout);
                            let first = s.lines().next()?.trim_start_matches("NVIM v");
                            Some(first.split('-').next()?.to_string())
                        })
                        .unwrap_or_else(|| "0.13.0".to_string());
                    let arch_attr = if cfg!(target_arch = "aarch64") {
                        "arm64"
                    } else {
                        "x64"
                    };
                    let icon_path = root.join("runtime").join("neovim.ico");
                    let wxs_path = out_dir.join("_nvim_installer.wxs");
                    let wxs_content = format!(
                        "<Wix xmlns=\"http://wixtoolset.org/schemas/v4/wxs\">\n\
                         \x20 <Package Name=\"Neovim\"\n\
                         \x20          Manufacturer=\"neovim.io\"\n\
                         \x20          Version=\"{ver}\"\n\
                         \x20          UpgradeCode=\"207A1A70-7B0C-418A-A153-CA6883E38F4D\"\n\
                         \x20          Scope=\"perMachine\">\n\
                         \x20   <MajorUpgrade DowngradeErrorMessage=\"A newer version of [ProductName] is already installed.\" />\n\
                         \x20   <MediaTemplate EmbedCab=\"yes\" />\n\
                         \x20   <Icon Id=\"neovim.ico\" SourceFile=\"{}\" />\n\
                         \x20   <Property Id=\"ARPPRODUCTICON\" Value=\"neovim.ico\" />\n\
                         \x20   <StandardDirectory Id=\"ProgramFiles6432Folder\">\n\
                         \x20     <Directory Id=\"INSTALLFOLDER\" Name=\"Neovim\">\n\
                         \x20       <Files Include=\"{}\\**\" />\n\
                         \x20     </Directory>\n\
                         \x20   </StandardDirectory>\n\
                         \x20 </Package>\n\
                         </Wix>\n",
                        icon_path.display(),
                        stage_root.display(),
                    );
                    fs::write(&wxs_path, wxs_content).unwrap();
                    let mut wix_cmd = Command::new(&wix);
                    wix_cmd
                        .current_dir(&out_dir)
                        .env("DOTNET_ROLL_FORWARD", "Major")
                        .args(["build", "-arch", arch_attr, "-o"])
                        .arg(&msi_path)
                        .arg(&wxs_path);
                    run_status(&mut wix_cmd, "wix build msi");
                    let _ = fs::remove_file(&wxs_path);
                    let _ = fs::remove_file(out_dir.join(format!("{pkg_name}.wixpdb")));
                    eprintln!("[dist] Packaged {}", msi_path.display());
                }
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

                if cfg!(target_os = "linux") {
                    let uname_arch = if cfg!(target_arch = "aarch64") {
                        "aarch64"
                    } else {
                        "x86_64"
                    };
                    let arch_output = if cfg!(target_arch = "aarch64") {
                        "arm64"
                    } else {
                        "x86_64"
                    };
                    let appdir = out_dir.join("org.neovim.nvim.AppDir");
                    if appdir.exists() {
                        let _ = fs::remove_dir_all(&appdir);
                    }
                    let appdir_usr = appdir.join("usr");
                    deps_build::copy_dir_all(&stage_root, &appdir_usr).unwrap();
                    let metainfo_dir = appdir_usr.join("share").join("metainfo");
                    fs::create_dir_all(&metainfo_dir).unwrap();
                    fs::copy(
                        root.join("runtime").join("org.neovim.nvim.appdata.xml"),
                        metainfo_dir.join("org.neovim.nvim.appdata.xml"),
                    )
                    .unwrap();
                    let apprun = appdir.join("AppRun");
                    fs::write(
                        &apprun,
                        "#!/bin/bash\n\nunset ARGV0\nexec \"$(dirname \"$(readlink -f \"${0}\")\")/usr/bin/nvim\" ${@+\"$@\"}\n",
                    )
                    .unwrap();
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let mut perms = fs::metadata(&apprun).unwrap().permissions();
                        perms.set_mode(0o755);
                        fs::set_permissions(&apprun, perms).unwrap();
                    }

                    let dl_dir = root.join(".deps").join("build").join("downloads");
                    fs::create_dir_all(&dl_dir).unwrap();
                    let linuxdeploy = dl_dir.join(format!("linuxdeploy-{uname_arch}.AppImage"));
                    if !linuxdeploy.exists() {
                        let url = format!(
                            "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-{uname_arch}.AppImage"
                        );
                        let mut dl_cmd = Command::new("curl");
                        dl_cmd
                            .args(["-sfL", "--retry", "3", &url, "-o"])
                            .arg(&linuxdeploy);
                        run_status(&mut dl_cmd, "download linuxdeploy");
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let mut perms = fs::metadata(&linuxdeploy).unwrap().permissions();
                            perms.set_mode(0o755);
                            fs::set_permissions(&linuxdeploy, perms).unwrap();
                        }
                    }

                    let appimage_name = format!("nvim-linux-{arch_output}.appimage");
                    let appimage_out = out_dir.join(&appimage_name);
                    if appimage_out.exists() {
                        let _ = fs::remove_file(&appimage_out);
                    }
                    let ver_out = Command::new(stage_root.join("bin").join("nvim"))
                        .arg("--version")
                        .output()
                        .ok()
                        .map(|o| {
                            String::from_utf8_lossy(&o.stdout)
                                .lines()
                                .next()
                                .unwrap_or("v0.13.0-dev")
                                .trim_start_matches("NVIM ")
                                .to_string()
                        })
                        .unwrap_or_else(|| "v0.13.0-dev".to_string());

                    let mut ld_cmd = Command::new(&linuxdeploy);
                    ld_cmd
                        .current_dir(&out_dir)
                        .env("ARCH", uname_arch)
                        .env("OUTPUT", &appimage_name)
                        .env("LDAI_OUTPUT", &appimage_name)
                        .env("VERSION", &ver_out)
                        .env("LINUXDEPLOY_OUTPUT_VERSION", &ver_out)
                        .env("APPIMAGE_EXTRACT_AND_RUN", "1")
                        .env("LDAI_NO_APPSTREAM", "1");
                    if let Ok(tag) = env::var("APPIMAGE_TAG")
                        && !tag.is_empty()
                    {
                        let update_info = format!(
                            "gh-releases-zsync|freddiehaddad|neovim|{tag}|{appimage_name}.zsync"
                        );
                        ld_cmd.env("UPDATE_INFORMATION", &update_info);
                        ld_cmd.env("LDAI_UPDATE_INFORMATION", &update_info);
                    }
                    ld_cmd
                        .arg("--appdir")
                        .arg(&appdir)
                        .arg("-d")
                        .arg(root.join("runtime").join("org.neovim.nvim.desktop"))
                        .arg("-i")
                        .arg(root.join("runtime").join("nvim.png"))
                        .args(["--output", "appimage"]);
                    run_status(&mut ld_cmd, "linuxdeploy appimage");
                    let _ = fs::remove_dir_all(&appdir);
                    eprintln!("[dist] Packaged {}", appimage_out.display());
                }
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
                    .arg(root.join("src").join("gen").join("gen_help_html.lua"));
                if subcmd == "lintdocurls" {
                    cmd.arg("true");
                }
                run_status(&mut cmd, subcmd);
            }
        }
        "lintrust" => {
            let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
            let mut fmt_cmd = Command::new(&cargo);
            fmt_cmd
                .current_dir(&root)
                .args(["fmt", "--all", "--", "--check"]);
            run_status(&mut fmt_cmd, "cargo fmt --check");

            let deps_prefix = default_deps_prefix(&root);
            deps_build::ensure_deps(&root, &deps_prefix, prefer_lua, false);
            let mut cmd = Command::new(cargo);
            cmd.current_dir(&root).args([
                "clippy",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--",
                "-D",
                "warnings",
            ]);
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
            cmd1.args(["runtime", "src", "test", "contrib"]);
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
        "linterrcodes" => {
            run_linterrcodes(&root);
        }
        "lintcommit" => {
            run_lintcommit(&root);
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
        "testrust" | "unittest" => {
            let deps_prefix = default_deps_prefix(&root);
            deps_build::ensure_deps(&root, &deps_prefix, prefer_lua, false);
            let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
            let mut cmd = Command::new(cargo);
            cmd.current_dir(&root).args(["test", "--lib"]);
            if release {
                cmd.arg("--release");
            }
            if prefer_lua {
                cmd.args(["--features", "vendor-bit"]);
            }
            for a in &extra_args {
                cmd.arg(a);
            }
            run_status(&mut cmd, "cargo test");
        }
        "functionaltest" | "benchmark" => {
            let profile_dir = build_cargo_bins(&root, release, prefer_lua);
            let nvim_bin = stage_build_dir(&root, &profile_dir, &build_dir);
            let test_type = match subcmd {
                "functionaltest" => "functional",
                _ => "benchmark",
            };
            run_lua_tests(&root, &build_dir, &nvim_bin, test_type, &extra_args);
        }
        "oldtest" => {
            let profile_dir = build_cargo_bins(&root, release, prefer_lua);
            let nvim_bin = stage_build_dir(&root, &profile_dir, &build_dir);
            let norm_slashes = |p: &Path| p.to_string_lossy().replace('\\', "/");
            let mut cmd = Command::new(&nvim_bin);
            cmd.current_dir(&root)
                .env("VIMRUNTIME", norm_slashes(&root.join("runtime")))
                .env("BUILD_DIR", norm_slashes(&build_dir))
                .env("XDG_CONFIG_HOME", norm_slashes(&root))
                .env("NVIM_TEST_ARG0", norm_slashes(&nvim_bin))
                .env_remove("NoDefaultCurrentDirectoryInExePath")
                .args(["--clean", "-l"])
                .arg(root.join("test").join("old").join("runner.lua"));
            if cfg!(target_os = "windows")
                && let Ok(path) = env::var("PATH")
            {
                let git_usr_bin = Path::new(r"C:\Program Files\Git\usr\bin");
                if git_usr_bin.is_dir() && !path.to_ascii_lowercase().contains(r"git\usr\bin") {
                    cmd.env(
                        "PATH",
                        format!(r"C:\Windows\System32;{};{path}", git_usr_bin.display()),
                    );
                }
            }
            for a in &extra_args {
                cmd.arg(a);
            }
            run_status(&mut cmd, "oldtest");
        }
        "test" => {
            let exe = env::current_exe().unwrap();
            for step in ["unittest", "functionaltest"] {
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
