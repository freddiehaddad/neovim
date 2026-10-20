use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=DEPS_PREFIX");
    println!("cargo:rerun-if-env-changed=VTERM_TEST_FILE");
    println!("cargo:rerun-if-env-changed=NVIM_VERSION_LONG");
    println!("cargo:rerun-if-env-changed=NVIM_VERSION_BUILD");
    println!("cargo:rerun-if-env-changed=NVIM_BUILD_TYPE");
    println!("cargo:rerun-if-env-changed=NVIM_VERSION_CFLAGS");
    println!("cargo:rerun-if-env-changed=NVIM_DEFAULT_VIM_DIR");
    println!("cargo:rerun-if-env-changed=NVIM_DEFAULT_VIMRUNTIME_DIR");
    println!("cargo:rerun-if-env-changed=NVIM_DEFAULT_LIB_DIR");
    println!("cargo:rerun-if-env-changed=NVIM_EXITFREE");
    println!("cargo:rerun-if-env-changed=NVIM_HAVE_UNIBILIUM");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    if env::var("VTERM_TEST_FILE").is_err() {
        let vterm_test_file = manifest_dir
            .join("build")
            .join("test")
            .join("vterm_test_output");
        println!(
            "cargo:rustc-env=VTERM_TEST_FILE={}",
            vterm_test_file.display()
        );
    }

    if env::var("NVIM_VERSION_LONG").is_err() {
        let mut version = String::from("v0.13.0-dev");
        let mut build = String::new();
        if let Ok(out) = Command::new("git")
            .args([
                "--git-dir",
                manifest_dir.join(".git").to_str().unwrap_or(".git"),
                "--work-tree",
                manifest_dir.to_str().unwrap_or("."),
                "describe",
                "--first-parent",
                "--dirty",
                "--always",
            ])
            .output()
            && out.status.success()
        {
            let tag = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !tag.is_empty() {
                if let Some(idx) = tag.rfind('-') {
                    build = tag[idx + 1..].to_string();
                } else {
                    build = tag.clone();
                }
                let rest = tag
                    .strip_prefix("v0.13.0-")
                    .or_else(|| {
                        let first_dash = tag.find('-')?;
                        Some(&tag[first_dash + 1..])
                    })
                    .unwrap_or(&tag);
                let formatted = if let Some((commits, hash)) = rest.split_once('-') {
                    format!("{commits}+{hash}")
                } else {
                    rest.to_string()
                };
                version = format!("v0.13.0-dev-{formatted}");
            }
        }
        println!("cargo:rustc-env=NVIM_VERSION_LONG=NVIM {version}");
        println!("cargo:rustc-env=NVIM_VERSION_BUILD={build}");
    }

    let deps_prefix = env::var("DEPS_PREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join(".deps").join("usr"));
    let deps_lib = deps_prefix.join("lib");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    if target_os == "windows" && target_env == "msvc" {
        println!(
            "cargo:rustc-link-arg-bin=nvim=/LIBPATH:{}",
            deps_lib.display()
        );
        for lib in [
            "luv.lib",
            "libiconv.lib",
            "tree-sitter.lib",
            "utf8proc_static.lib",
            "libintl.lib",
            "unibilium.lib",
            "lua51.lib",
            "libuv.lib",
            "netapi32.lib",
            "bcrypt.lib",
            "ntdll.lib",
            "iphlpapi.lib",
            "psapi.lib",
            "ws2_32.lib",
            "userenv.lib",
            "dbghelp.lib",
            "ole32.lib",
            "shell32.lib",
            "advapi32.lib",
            "user32.lib",
            "gdi32.lib",
        ] {
            println!("cargo:rustc-link-arg-bin=nvim={lib}");
        }
        println!("cargo:rustc-link-arg-bin=nvim=/DEFAULTLIB:msvcrt");
        println!("cargo:rustc-link-arg-bin=nvim=/DEFAULTLIB:ucrt");
        println!("cargo:rustc-link-arg-bin=nvim=/NODEFAULTLIB:libcmt");
        println!("cargo:rustc-link-arg-bin=nvim=/NODEFAULTLIB:libucrt");
        let manifest_path = manifest_dir
            .join("src")
            .join("nvim")
            .join("os")
            .join("nvim.manifest");
        println!("cargo:rustc-link-arg-bin=nvim=/MANIFEST:EMBED,ID=1");
        println!(
            "cargo:rustc-link-arg-bin=nvim=/MANIFESTINPUT:{}",
            manifest_path.display()
        );
    } else if target_os != "windows" {
        println!("cargo:rustc-link-arg-bin=nvim=-rdynamic");
        println!("cargo:rustc-link-arg-bin=nvim=-L{}", deps_lib.display());
        for archive in [
            "libluv.a",
            "libtree-sitter.a",
            "libutf8proc.a",
            "libunibilium.a",
            "libluajit-5.1.a",
            "libuv.a",
        ] {
            let p = deps_lib.join(archive);
            if p.exists() {
                println!("cargo:rustc-link-arg-bin=nvim={}", p.display());
            }
        }
        for syslib in ["-lm", "-ldl", "-lpthread"] {
            println!("cargo:rustc-link-arg-bin=nvim={syslib}");
        }
        if target_os == "linux" {
            println!("cargo:rustc-link-arg-bin=nvim=-lutil");
        } else if target_os == "macos" {
            println!("cargo:rustc-link-arg-bin=nvim=-liconv");
            println!("cargo:rustc-link-arg-bin=nvim=-Wl,-export_dynamic");
            println!("cargo:rustc-link-arg-bin=nvim=-framework");
            println!("cargo:rustc-link-arg-bin=nvim=CoreServices");
        }
    }
}
