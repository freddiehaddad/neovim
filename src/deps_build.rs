use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256_process_block(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }
    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

pub fn sha256_reader<R: Read>(mut reader: R) -> io::Result<String> {
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut total_bytes: u64 = 0;
    let mut buf = [0u8; 64];
    let mut buf_len = 0usize;

    loop {
        let n = reader.read(&mut buf[buf_len..])?;
        if n == 0 {
            break;
        }
        buf_len += n;
        total_bytes += n as u64;
        if buf_len == 64 {
            sha256_process_block(&mut state, &buf);
            buf_len = 0;
        }
    }

    let mut pad = [0u8; 128];
    pad[..buf_len].copy_from_slice(&buf[..buf_len]);
    pad[buf_len] = 0x80;
    let total_bits = total_bytes * 8;
    if buf_len < 56 {
        pad[56..64].copy_from_slice(&total_bits.to_be_bytes());
        let block: &[u8; 64] = pad[0..64].try_into().unwrap();
        sha256_process_block(&mut state, block);
    } else {
        pad[120..128].copy_from_slice(&total_bits.to_be_bytes());
        let block1: &[u8; 64] = pad[0..64].try_into().unwrap();
        let block2: &[u8; 64] = pad[64..128].try_into().unwrap();
        sha256_process_block(&mut state, block1);
        sha256_process_block(&mut state, block2);
    }

    let mut hex = String::with_capacity(64);
    for word in state {
        for byte in word.to_be_bytes() {
            use std::fmt::Write;
            let _ = write!(hex, "{byte:02x}");
        }
    }
    Ok(hex)
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let file = fs::File::open(path)?;
    sha256_reader(io::BufReader::new(file))
}

fn parse_deps_txt(manifest_dir: &Path) -> HashMap<String, String> {
    let deps_path = manifest_dir.join("deps.txt");
    let text = fs::read_to_string(&deps_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", deps_path.display()));
    let mut map = HashMap::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once(char::is_whitespace) {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    map
}

#[derive(Clone, Default)]
struct CompilerEnv {
    vars: Vec<(String, String)>,
}

impl CompilerEnv {
    fn detect() -> Self {
        if !cfg!(target_os = "windows") {
            return Self::default();
        }
        if Command::new("cl.exe").arg("/?").output().is_ok() {
            return Self::default();
        }
        let pf86 =
            env::var("ProgramFiles(x86)").unwrap_or_else(|_| r"C:\Program Files (x86)".to_string());
        let vswhere = PathBuf::from(pf86)
            .join("Microsoft Visual Studio")
            .join("Installer")
            .join("vswhere.exe");
        if !vswhere.exists() {
            panic!(
                "cl.exe not in PATH and vswhere.exe not found at {}",
                vswhere.display()
            );
        }
        let out = Command::new(&vswhere)
            .args([
                "-latest",
                "-products",
                "*",
                "-requires",
                "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                "-property",
                "installationPath",
            ])
            .output()
            .expect("failed to execute vswhere.exe");
        let install_dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert!(
            !install_dir.is_empty(),
            "vswhere did not find a Visual Studio C++ installation"
        );
        let vcvars_dir = PathBuf::from(install_dir)
            .join("VC")
            .join("Auxiliary")
            .join("Build");
        let vcvars = if cfg!(target_arch = "aarch64") {
            let native = vcvars_dir.join("vcvarsarm64.bat");
            if native.exists() {
                native
            } else {
                vcvars_dir.join("vcvarsx86_arm64.bat")
            }
        } else {
            vcvars_dir.join("vcvars64.bat")
        };
        assert!(
            vcvars.exists(),
            "vcvars script not found at {}",
            vcvars.display()
        );

        #[cfg(windows)]
        let env_out = {
            use std::os::windows::process::CommandExt;
            let raw_cmd = format!("\"call \"{}\" >nul && set\"", vcvars.display());
            Command::new("cmd.exe")
                .arg("/d")
                .arg("/s")
                .arg("/c")
                .raw_arg(&raw_cmd)
                .output()
                .expect("failed to run vcvars64.bat")
        };
        #[cfg(not(windows))]
        let env_out = {
            let cmd_str = format!("call \"{}\" >nul && set", vcvars.display());
            Command::new("cmd.exe")
                .args(["/d", "/c", &cmd_str])
                .output()
                .expect("failed to run vcvars64.bat")
        };
        assert!(
            env_out.status.success(),
            "vcvars64.bat failed: {}",
            String::from_utf8_lossy(&env_out.stderr)
        );
        let mut vars = Vec::new();
        for line in String::from_utf8_lossy(&env_out.stdout).lines() {
            if let Some((k, v)) = line.split_once('=') {
                vars.push((k.to_string(), v.to_string()));
            }
        }
        Self { vars }
    }

    fn apply<'a>(&self, cmd: &'a mut Command) -> &'a mut Command {
        for (k, v) in &self.vars {
            cmd.env(k, v);
        }
        cmd.env_remove("NoDefaultCurrentDirectoryInExePath")
    }
}

fn run_cmd(cmd: &mut Command, desc: &str) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to start {desc}: {e}"));
    assert!(status.success(), "{desc} failed with status {status}");
}

fn download_and_extract(
    deps: &HashMap<String, String>,
    name: &str,
    downloads_dir: &Path,
    src_dir: &Path,
    is_zip: bool,
) {
    let key_upper = name.to_uppercase();
    let url = deps
        .get(&format!("{key_upper}_URL"))
        .unwrap_or_else(|| panic!("missing {key_upper}_URL in deps.txt"));
    let expected_sha = deps
        .get(&format!("{key_upper}_SHA256"))
        .unwrap_or_else(|| panic!("missing {key_upper}_SHA256 in deps.txt"));
    let ignore_sha = matches!(
        env::var("DEPS_IGNORE_SHA").as_deref(),
        Ok("1" | "TRUE" | "true" | "ON" | "on")
    );

    let dl_subdir = downloads_dir.join(name);
    fs::create_dir_all(&dl_subdir).expect("failed to create download dir");
    let filename = url.rsplit('/').next().unwrap_or("archive.tar.gz");
    let archive_path = dl_subdir.join(filename);

    let mut need_download = !archive_path.exists();
    if !need_download && !ignore_sha {
        match sha256_file(&archive_path) {
            Ok(actual) if actual.eq_ignore_ascii_case(expected_sha) => {}
            _ => {
                let _ = fs::remove_file(&archive_path);
                need_download = true;
            }
        }
    }

    if need_download {
        eprintln!("[deps] Downloading {name} from {url}");
        let mut cmd = Command::new("curl");
        cmd.args(["-sfL", "--retry", "3", url, "-o"])
            .arg(&archive_path);
        run_cmd(&mut cmd, &format!("curl download {name}"));
        if !ignore_sha {
            let actual = sha256_file(&archive_path)
                .unwrap_or_else(|e| panic!("failed to hash {}: {e}", archive_path.display()));
            assert!(
                actual.eq_ignore_ascii_case(expected_sha),
                "SHA256 mismatch for {name}: expected {expected_sha}, got {actual}"
            );
        }
    }

    let stamp_file = src_dir.join(".extracted-sha256");
    if stamp_file.exists()
        && fs::read_to_string(&stamp_file).ok().as_deref() == Some(expected_sha.as_str())
    {
        return;
    }

    if src_dir.exists() {
        fs::remove_dir_all(src_dir).expect("failed to clean existing src_dir");
    }
    fs::create_dir_all(src_dir).expect("failed to create src_dir");

    let mut tar_cmd = Command::new("tar");
    if is_zip {
        tar_cmd.arg("-xf").arg(&archive_path).arg("-C").arg(src_dir);
    } else {
        tar_cmd
            .arg("-xzf")
            .arg(&archive_path)
            .arg("-C")
            .arg(src_dir)
            .arg("--strip-components=1");
    }
    run_cmd(&mut tar_cmd, &format!("tar extract {name}"));
    fs::write(stamp_file, expected_sha).expect("failed to write .extracted-sha256");
}

pub fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}

fn compile_c_static_lib(
    c_env: &CompilerEnv,
    sources: &[PathBuf],
    includes: &[PathBuf],
    defines: &[&str],
    obj_dir: &Path,
    out_lib: &Path,
) {
    fs::create_dir_all(obj_dir).expect("failed to create obj_dir");
    if let Some(parent) = out_lib.parent() {
        fs::create_dir_all(parent).expect("failed to create lib parent dir");
    }

    let mut obj_files = Vec::with_capacity(sources.len());
    for (idx, src) in sources.iter().enumerate() {
        let stem = src.file_stem().unwrap().to_string_lossy();
        let ext = if cfg!(target_os = "windows") {
            "obj"
        } else {
            "o"
        };
        obj_files.push((src.clone(), obj_dir.join(format!("{idx}_{stem}.{ext}"))));
    }

    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let chunk_size = obj_files.len().div_ceil(threads).max(1);

    std::thread::scope(|s| {
        for chunk in obj_files.chunks(chunk_size) {
            s.spawn(move || {
                for (src, obj) in chunk {
                    if cfg!(target_os = "windows") {
                        let mut cmd = Command::new("cl.exe");
                        c_env.apply(&mut cmd);
                        cmd.args(["/nologo", "/O2", "/MD", "/W0", "/utf-8", "/std:c11", "/c"]);
                        for inc in includes {
                            cmd.arg(format!("/I{}", inc.display()));
                        }
                        for def in defines {
                            cmd.arg(format!("/D{def}"));
                        }
                        cmd.arg(src).arg(format!("/Fo{}", obj.display()));
                        run_cmd(&mut cmd, &format!("cl.exe {}", src.display()));
                    } else {
                        let cc = env::var("CC").unwrap_or_else(|_| "cc".to_string());
                        let mut cmd = Command::new(&cc);
                        cmd.args([
                            "-O2",
                            "-g",
                            "-fPIC",
                            "-w",
                            "-std=gnu11",
                            "-fvisibility=hidden",
                            "-c",
                        ]);
                        for inc in includes {
                            cmd.arg("-I").arg(inc);
                        }
                        for def in defines {
                            cmd.arg(format!("-D{def}"));
                        }
                        cmd.arg(src).arg("-o").arg(obj);
                        run_cmd(&mut cmd, &format!("{cc} {}", src.display()));
                    }
                }
            });
        }
    });

    if out_lib.exists() {
        let _ = fs::remove_file(out_lib);
    }

    if cfg!(target_os = "windows") {
        let mut cmd = Command::new("lib.exe");
        c_env.apply(&mut cmd);
        cmd.arg("/nologo")
            .arg(format!("/OUT:{}", out_lib.display()));
        for (_, obj) in &obj_files {
            cmd.arg(obj);
        }
        run_cmd(&mut cmd, &format!("lib.exe {}", out_lib.display()));
    } else {
        let ar = env::var("AR").unwrap_or_else(|_| "ar".to_string());
        let mut cmd = Command::new(&ar);
        cmd.arg("crs").arg(out_lib);
        for (_, obj) in &obj_files {
            cmd.arg(obj);
        }
        run_cmd(&mut cmd, &format!("{ar} {}", out_lib.display()));
    }
}

fn compile_c_shared_module(
    c_env: &CompilerEnv,
    sources: &[PathBuf],
    includes: &[PathBuf],
    obj_dir: &Path,
    out_mod: &Path,
) {
    fs::create_dir_all(obj_dir).expect("failed to create obj_dir");
    if let Some(parent) = out_mod.parent() {
        fs::create_dir_all(parent).expect("failed to create parser dir");
    }

    if cfg!(target_os = "windows") {
        let stem = out_mod.file_stem().unwrap().to_string_lossy();
        let mut cmd = Command::new("cl.exe");
        c_env.apply(&mut cmd);
        cmd.args(["/nologo", "/O2", "/MD", "/W0", "/utf-8", "/std:c11"]);
        for inc in includes {
            cmd.arg(format!("/I{}", inc.display()));
        }
        for src in sources {
            cmd.arg(src);
        }
        let fo_dir = format!("{}\\", obj_dir.display());
        let implib = obj_dir.join(format!("{stem}.lib"));
        let pdb = obj_dir.join(format!("{stem}.pdb"));
        cmd.arg("/LD")
            .arg(format!("/Fe:{}", out_mod.display()))
            .arg(format!("/Fo:{fo_dir}"))
            .arg("/link")
            .arg(format!("/IMPLIB:{}", implib.display()))
            .arg(format!("/PDB:{}", pdb.display()));
        run_cmd(&mut cmd, &format!("cl.exe shared {}", out_mod.display()));
    } else {
        let cc = env::var("CC").unwrap_or_else(|_| "cc".to_string());
        let mut cmd = Command::new(&cc);
        cmd.args(["-O2", "-fPIC", "-w"]);
        if cfg!(target_os = "macos") {
            cmd.args(["-dynamiclib", "-Wl,-undefined,dynamic_lookup"]);
        } else {
            cmd.arg("-shared");
        }
        for inc in includes {
            cmd.arg("-I").arg(inc);
        }
        for src in sources {
            cmd.arg(src);
        }
        cmd.arg("-o").arg(out_mod);
        run_cmd(&mut cmd, &format!("{cc} shared {}", out_mod.display()));
    }
}

fn build_libuv(
    c_env: &CompilerEnv,
    deps: &HashMap<String, String>,
    downloads_dir: &Path,
    build_src_dir: &Path,
    deps_prefix: &Path,
) {
    let out_lib = if cfg!(target_os = "windows") {
        deps_prefix.join("lib").join("libuv.lib")
    } else {
        deps_prefix.join("lib").join("libuv.a")
    };
    let src_dir = build_src_dir.join("libuv");
    download_and_extract(deps, "libuv", downloads_dir, &src_dir, false);

    eprintln!("[deps] Building libuv...");
    let mut sources: Vec<PathBuf> = [
        "src/fs-poll.c",
        "src/idna.c",
        "src/inet.c",
        "src/random.c",
        "src/strscpy.c",
        "src/strtok.c",
        "src/thread-common.c",
        "src/threadpool.c",
        "src/timer.c",
        "src/uv-common.c",
        "src/uv-data-getter-setters.c",
        "src/version.c",
    ]
    .iter()
    .map(|p| src_dir.join(p))
    .collect();

    let mut defines: Vec<&str> = Vec::new();
    if cfg!(target_os = "windows") {
        defines.extend([
            "WIN32_LEAN_AND_MEAN",
            "_WIN32_WINNT=0x0A00",
            "_CRT_DECLARE_NONSTDC_NAMES=0",
        ]);
        for p in [
            "src/win/async.c",
            "src/win/core.c",
            "src/win/detect-wakeup.c",
            "src/win/dl.c",
            "src/win/error.c",
            "src/win/fs.c",
            "src/win/fs-event.c",
            "src/win/getaddrinfo.c",
            "src/win/getnameinfo.c",
            "src/win/handle.c",
            "src/win/loop-watcher.c",
            "src/win/pipe.c",
            "src/win/thread.c",
            "src/win/poll.c",
            "src/win/process.c",
            "src/win/process-stdio.c",
            "src/win/signal.c",
            "src/win/snprintf.c",
            "src/win/stream.c",
            "src/win/tcp.c",
            "src/win/tty.c",
            "src/win/udp.c",
            "src/win/util.c",
            "src/win/winapi.c",
            "src/win/winsock.c",
        ] {
            sources.push(src_dir.join(p));
        }
    } else {
        defines.extend(["_FILE_OFFSET_BITS=64", "_LARGEFILE_SOURCE"]);
        for p in [
            "src/unix/async.c",
            "src/unix/core.c",
            "src/unix/dl.c",
            "src/unix/fs.c",
            "src/unix/getaddrinfo.c",
            "src/unix/getnameinfo.c",
            "src/unix/loop-watcher.c",
            "src/unix/loop.c",
            "src/unix/pipe.c",
            "src/unix/poll.c",
            "src/unix/process.c",
            "src/unix/random-devurandom.c",
            "src/unix/signal.c",
            "src/unix/stream.c",
            "src/unix/tcp.c",
            "src/unix/thread.c",
            "src/unix/tty.c",
            "src/unix/udp.c",
        ] {
            sources.push(src_dir.join(p));
        }
        if cfg!(target_os = "linux") {
            defines.extend(["_GNU_SOURCE", "_POSIX_C_SOURCE=200112"]);
            for p in [
                "src/unix/proctitle.c",
                "src/unix/linux.c",
                "src/unix/procfs-exepath.c",
                "src/unix/random-getrandom.c",
                "src/unix/random-sysctl-linux.c",
            ] {
                sources.push(src_dir.join(p));
            }
        } else if cfg!(target_os = "macos") {
            defines.extend(["_DARWIN_UNLIMITED_SELECT=1", "_DARWIN_USE_64_BIT_INODE=1"]);
            for p in [
                "src/unix/proctitle.c",
                "src/unix/bsd-ifaddrs.c",
                "src/unix/kqueue.c",
                "src/unix/random-getentropy.c",
                "src/unix/darwin-proctitle.c",
                "src/unix/darwin.c",
                "src/unix/fsevents.c",
            ] {
                sources.push(src_dir.join(p));
            }
        } else if cfg!(any(
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        )) {
            for p in [
                "src/unix/posix-hrtime.c",
                "src/unix/bsd-proctitle.c",
                "src/unix/bsd-ifaddrs.c",
                "src/unix/kqueue.c",
            ] {
                sources.push(src_dir.join(p));
            }
            if cfg!(target_os = "freebsd") || cfg!(target_os = "dragonfly") {
                sources.push(src_dir.join("src/unix/freebsd.c"));
            }
            if cfg!(target_os = "freebsd") {
                sources.push(src_dir.join("src/unix/random-getrandom.c"));
            }
            if cfg!(target_os = "openbsd") {
                sources.push(src_dir.join("src/unix/random-getentropy.c"));
                sources.push(src_dir.join("src/unix/openbsd.c"));
            }
            if cfg!(target_os = "netbsd") {
                sources.push(src_dir.join("src/unix/netbsd.c"));
            }
        }
    }

    let includes = [src_dir.join("include"), src_dir.join("src")];
    let obj_dir = build_src_dir.join("libuv-obj");
    compile_c_static_lib(c_env, &sources, &includes, &defines, &obj_dir, &out_lib);
    copy_dir_all(&src_dir.join("include"), &deps_prefix.join("include"))
        .expect("failed to copy libuv headers");
}

fn find_make_prg() -> String {
    if cfg!(any(
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )) && Command::new("gmake").arg("--version").output().is_ok()
    {
        return "gmake".to_string();
    }
    "make".to_string()
}

fn build_luajit(
    c_env: &CompilerEnv,
    deps: &HashMap<String, String>,
    downloads_dir: &Path,
    build_src_dir: &Path,
    deps_prefix: &Path,
) {
    let src_dir = build_src_dir.join("luajit");
    download_and_extract(deps, "luajit", downloads_dir, &src_dir, false);

    eprintln!("[deps] Building luajit...");
    let lj_src = src_dir.join("src");
    if cfg!(target_os = "windows") {
        let mut cmd = Command::new("cmd.exe");
        c_env.apply(&mut cmd);
        cmd.current_dir(&lj_src).args(["/d", "/c", "msvcbuild.bat"]);
        run_cmd(&mut cmd, "luajit msvcbuild.bat");

        let bin_dir = deps_prefix.join("bin");
        let lib_dir = deps_prefix.join("lib");
        let inc_dir = deps_prefix.join("include").join("luajit-2.1");
        let jit_dir = deps_prefix.join("share").join("luajit-2.1").join("jit");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::create_dir_all(&lib_dir).unwrap();
        fs::create_dir_all(&inc_dir).unwrap();
        fs::create_dir_all(&jit_dir).unwrap();

        fs::copy(lj_src.join("luajit.exe"), bin_dir.join("luajit.exe")).unwrap();
        fs::copy(lj_src.join("lua51.dll"), bin_dir.join("lua51.dll")).unwrap();
        fs::copy(lj_src.join("lua51.lib"), lib_dir.join("lua51.lib")).unwrap();
        fs::copy(lj_src.join("lua51.lib"), lib_dir.join("luajit.lib")).unwrap();
        for entry in fs::read_dir(&lj_src).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("h") {
                fs::copy(&path, inc_dir.join(entry.file_name())).unwrap();
            }
        }
        copy_dir_all(&lj_src.join("jit"), &jit_dir).unwrap();
    } else {
        let make = find_make_prg();
        let cc = env::var("CC").unwrap_or_else(|_| "cc".to_string());
        let jobs = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let mut cmd = Command::new(&make);
        cmd.current_dir(&src_dir)
            .arg(format!("-j{jobs}"))
            .arg(format!("CC={cc}"))
            .arg(format!("PREFIX={}", deps_prefix.display()))
            .arg("CFLAGS=-fPIC")
            .arg("CFLAGS+=-DLUA_USE_APICHECK")
            .arg("CFLAGS+=-funwind-tables")
            .arg("CCDEBUG+=-g")
            .arg("Q=");
        if cfg!(target_os = "macos") {
            cmd.arg("CFLAGS+=-fno-stack-check");
            if env::var("MACOSX_DEPLOYMENT_TARGET").is_err()
                && let Ok(out) = Command::new("sw_vers").arg("-productVersion").output()
            {
                let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !ver.is_empty() {
                    cmd.arg(format!("MACOSX_DEPLOYMENT_TARGET={ver}"));
                }
            }
        }
        if cfg!(target_os = "openbsd") {
            cmd.arg("LDFLAGS=-lpthread -lc++abi");
        }
        cmd.arg("install");
        run_cmd(&mut cmd, "luajit make install");
    }
}

fn build_lua(
    c_env: &CompilerEnv,
    deps: &HashMap<String, String>,
    downloads_dir: &Path,
    build_src_dir: &Path,
    deps_prefix: &Path,
) {
    let src_dir = build_src_dir.join("lua");
    download_and_extract(deps, "lua", downloads_dir, &src_dir, false);

    eprintln!("[deps] Building PUC Lua 5.1...");
    let lua_src = src_dir.join("src");
    let luaconf_path = lua_src.join("luaconf.h");
    if let Ok(conf) = fs::read_to_string(&luaconf_path) {
        let patched = conf
            .lines()
            .filter(|l| !l.contains("#define LUA_USE_READLINE"))
            .collect::<Vec<_>>()
            .join("\n")
            .replace("/usr/local", &deps_prefix.to_string_lossy());
        fs::write(&luaconf_path, patched).unwrap();
    }

    let c_files = [
        "lapi.c",
        "lcode.c",
        "ldebug.c",
        "ldo.c",
        "ldump.c",
        "lfunc.c",
        "lgc.c",
        "llex.c",
        "lmem.c",
        "lobject.c",
        "lopcodes.c",
        "lparser.c",
        "lstate.c",
        "lstring.c",
        "ltable.c",
        "ltm.c",
        "lundump.c",
        "lvm.c",
        "lzio.c",
        "lauxlib.c",
        "lbaselib.c",
        "ldblib.c",
        "liolib.c",
        "lmathlib.c",
        "loslib.c",
        "ltablib.c",
        "lstrlib.c",
        "loadlib.c",
        "linit.c",
    ];
    let sources: Vec<PathBuf> = c_files.iter().map(|f| lua_src.join(f)).collect();
    let defines: Vec<&str> = if cfg!(target_os = "linux") {
        vec!["LUA_USE_LINUX"]
    } else if cfg!(target_os = "macos") {
        vec!["LUA_USE_MACOSX"]
    } else if cfg!(unix) {
        vec!["LUA_USE_POSIX", "LUA_USE_DLOPEN"]
    } else {
        vec![]
    };
    let out_lib = if cfg!(target_os = "windows") {
        deps_prefix.join("lib").join("lua51.lib")
    } else {
        deps_prefix.join("lib").join("liblua.a")
    };
    let obj_dir = build_src_dir.join("lua-obj");
    compile_c_static_lib(
        c_env,
        &sources,
        std::slice::from_ref(&lua_src),
        &defines,
        &obj_dir,
        &out_lib,
    );
    let inc_dir = deps_prefix.join("include");
    fs::create_dir_all(&inc_dir).unwrap();
    for h in ["lua.h", "luaconf.h", "lualib.h", "lauxlib.h", "lua.hpp"] {
        let p = lua_src.join(h);
        if p.exists() {
            fs::copy(&p, inc_dir.join(h)).unwrap();
        }
    }
}

fn build_luv(
    c_env: &CompilerEnv,
    deps: &HashMap<String, String>,
    downloads_dir: &Path,
    build_src_dir: &Path,
    deps_prefix: &Path,
) {
    let compat_dir = build_src_dir.join("lua_compat53");
    let luv_dir = build_src_dir.join("luv");
    download_and_extract(deps, "lua_compat53", downloads_dir, &compat_dir, false);
    download_and_extract(deps, "luv", downloads_dir, &luv_dir, false);

    eprintln!("[deps] Building luv...");
    let out_lib = if cfg!(target_os = "windows") {
        deps_prefix.join("lib").join("luv.lib")
    } else {
        deps_prefix.join("lib").join("libluv.a")
    };
    let sources = [luv_dir.join("src").join("luv.c")];
    let includes = [
        deps_prefix.join("include"),
        deps_prefix.join("include").join("luajit-2.1"),
        compat_dir.join("c-api"),
        luv_dir.join("src"),
    ];
    let defines: Vec<&str> = if cfg!(target_os = "windows") {
        vec!["LUA_BUILD_AS_DLL", "LUA_LIB"]
    } else {
        vec![]
    };
    let obj_dir = build_src_dir.join("luv-obj");
    compile_c_static_lib(c_env, &sources, &includes, &defines, &obj_dir, &out_lib);

    let inc_dir = deps_prefix.join("include").join("luv");
    fs::create_dir_all(&inc_dir).unwrap();
    for h in ["luv.h", "util.h", "lhandle.h", "lreq.h"] {
        fs::copy(luv_dir.join("src").join(h), inc_dir.join(h)).unwrap();
    }
}

fn build_treesitter(
    c_env: &CompilerEnv,
    deps: &HashMap<String, String>,
    downloads_dir: &Path,
    build_src_dir: &Path,
    deps_prefix: &Path,
) {
    let ts_dir = build_src_dir.join("treesitter");
    download_and_extract(deps, "treesitter", downloads_dir, &ts_dir, false);

    eprintln!("[deps] Building tree-sitter runtime...");
    let out_lib = if cfg!(target_os = "windows") {
        deps_prefix.join("lib").join("tree-sitter.lib")
    } else {
        deps_prefix.join("lib").join("libtree-sitter.a")
    };
    let sources = [ts_dir.join("lib").join("src").join("lib.c")];
    let includes = [
        ts_dir.join("lib").join("include"),
        ts_dir.join("lib").join("src"),
        ts_dir.join("lib").join("src").join("wasm"),
    ];
    let defines = [
        "_POSIX_C_SOURCE=200112L",
        "_DEFAULT_SOURCE",
        "_BSD_SOURCE",
        "_DARWIN_C_SOURCE",
    ];
    let obj_dir = build_src_dir.join("treesitter-obj");
    compile_c_static_lib(c_env, &sources, &includes, &defines, &obj_dir, &out_lib);

    let inc_dir = deps_prefix.join("include").join("tree_sitter");
    fs::create_dir_all(&inc_dir).unwrap();
    fs::copy(
        ts_dir
            .join("lib")
            .join("include")
            .join("tree_sitter")
            .join("api.h"),
        inc_dir.join("api.h"),
    )
    .unwrap();
}

fn build_treesitter_parsers(
    c_env: &CompilerEnv,
    deps: &HashMap<String, String>,
    downloads_dir: &Path,
    build_src_dir: &Path,
    deps_prefix: &Path,
) {
    let ext = if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };
    let parser_out_dir = deps_prefix.join("lib").join("nvim").join("parser");
    fs::create_dir_all(&parser_out_dir).unwrap();

    for lang in ["c", "diff", "lua", "vim", "vimdoc", "query"] {
        let dep_name = format!("treesitter_{lang}");
        let src_dir = build_src_dir.join(&dep_name);
        download_and_extract(deps, &dep_name, downloads_dir, &src_dir, false);
        eprintln!("[deps] Building tree-sitter-{lang} parser...");
        let c_src_dir = src_dir.join("src");
        let mut sources = vec![c_src_dir.join("parser.c")];
        let scanner = c_src_dir.join("scanner.c");
        if scanner.exists() {
            sources.push(scanner);
        }
        let includes = [c_src_dir];
        let obj_dir = build_src_dir.join(format!("{dep_name}-obj"));
        let out_mod = parser_out_dir.join(format!("{lang}.{ext}"));
        compile_c_shared_module(c_env, &sources, &includes, &obj_dir, &out_mod);
    }

    let md_dir = build_src_dir.join("treesitter_markdown");
    download_and_extract(deps, "treesitter_markdown", downloads_dir, &md_dir, false);
    for (sub_dir, out_name) in [
        ("tree-sitter-markdown", "markdown"),
        ("tree-sitter-markdown-inline", "markdown_inline"),
    ] {
        eprintln!("[deps] Building {out_name} parser...");
        let c_src_dir = md_dir.join(sub_dir).join("src");
        let sources = [c_src_dir.join("parser.c"), c_src_dir.join("scanner.c")];
        let includes = [c_src_dir];
        let obj_dir = build_src_dir.join(format!("treesitter_{out_name}-obj"));
        let out_mod = parser_out_dir.join(format!("{out_name}.{ext}"));
        compile_c_shared_module(c_env, &sources, &includes, &obj_dir, &out_mod);
    }
}

fn build_win32yank(
    deps: &HashMap<String, String>,
    downloads_dir: &Path,
    build_src_dir: &Path,
    deps_prefix: &Path,
) {
    if !cfg!(target_os = "windows") {
        return;
    }
    let src_dir = build_src_dir.join("win32yank");
    download_and_extract(deps, "win32yank_x86_64", downloads_dir, &src_dir, true);
    let bin_dir = deps_prefix.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    fs::copy(src_dir.join("win32yank.exe"), bin_dir.join("win32yank.exe"))
        .expect("failed to copy win32yank.exe");
}

fn deps_are_present(deps_prefix: &Path, prefer_lua: bool) -> bool {
    let lib_dir = deps_prefix.join("lib");
    let parser_dir = lib_dir.join("nvim").join("parser");
    let ext = if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };
    let required_libs: &[&str] = if cfg!(target_os = "windows") {
        &["libuv.lib", "lua51.lib", "luv.lib", "tree-sitter.lib"]
    } else if prefer_lua {
        &["libuv.a", "liblua.a", "libluv.a", "libtree-sitter.a"]
    } else {
        &["libuv.a", "libluajit-5.1.a", "libluv.a", "libtree-sitter.a"]
    };
    for lib in required_libs {
        if !lib_dir.join(lib).exists() {
            return false;
        }
    }
    for lang in [
        "c",
        "diff",
        "lua",
        "vim",
        "vimdoc",
        "query",
        "markdown",
        "markdown_inline",
    ] {
        if !parser_dir.join(format!("{lang}.{ext}")).exists() {
            return false;
        }
    }
    if cfg!(target_os = "windows") {
        let bin_dir = deps_prefix.join("bin");
        if !bin_dir.join("lua51.dll").exists() || !bin_dir.join("win32yank.exe").exists() {
            return false;
        }
    }
    true
}

pub fn ensure_deps(manifest_dir: &Path, deps_prefix: &Path, prefer_lua: bool, force: bool) {
    if matches!(
        env::var("USE_BUNDLED").as_deref(),
        Ok("0" | "OFF" | "off" | "FALSE" | "false")
    ) {
        return;
    }

    let deps_txt_path = manifest_dir.join("deps.txt");
    if !deps_txt_path.exists() {
        return;
    }
    let deps_sha = sha256_file(&deps_txt_path).unwrap_or_default();
    let stamp_expected = format!("{deps_sha}:prefer_lua={prefer_lua}:os={}", env::consts::OS);
    let stamp_path = deps_prefix.join(".deps-stamp");

    if !force && deps_are_present(deps_prefix, prefer_lua) {
        if let Ok(existing) = fs::read_to_string(&stamp_path)
            && existing.trim() == stamp_expected
        {
            return;
        }
        if !stamp_path.exists() {
            let _ = fs::write(&stamp_path, &stamp_expected);
            return;
        }
    }

    let deps = parse_deps_txt(manifest_dir);
    let deps_root = deps_prefix
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| manifest_dir.join(".deps"));
    let build_dir = deps_root.join("build");
    let downloads_dir = build_dir.join("downloads");
    let build_src_dir = build_dir.join("src");
    fs::create_dir_all(&downloads_dir).expect("failed to create downloads_dir");
    fs::create_dir_all(&build_src_dir).expect("failed to create build_src_dir");
    fs::create_dir_all(deps_prefix).expect("failed to create deps_prefix");

    let c_env = CompilerEnv::detect();

    build_libuv(&c_env, &deps, &downloads_dir, &build_src_dir, deps_prefix);
    if prefer_lua {
        build_lua(&c_env, &deps, &downloads_dir, &build_src_dir, deps_prefix);
    } else {
        build_luajit(&c_env, &deps, &downloads_dir, &build_src_dir, deps_prefix);
    }
    build_luv(&c_env, &deps, &downloads_dir, &build_src_dir, deps_prefix);
    build_treesitter(&c_env, &deps, &downloads_dir, &build_src_dir, deps_prefix);
    build_treesitter_parsers(&c_env, &deps, &downloads_dir, &build_src_dir, deps_prefix);
    build_win32yank(&deps, &downloads_dir, &build_src_dir, deps_prefix);

    let _ = fs::write(&stamp_path, &stamp_expected);
}
