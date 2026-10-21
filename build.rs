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
        println!("cargo:rustc-link-search=native={}", deps_lib.display());
        for lib in [
            "luv",
            "libiconv",
            "tree-sitter",
            "utf8proc_static",
            "libintl",
            "unibilium",
            "lua51",
            "libuv",
            "netapi32",
            "bcrypt",
            "ntdll",
            "iphlpapi",
            "psapi",
            "ws2_32",
            "userenv",
            "dbghelp",
            "ole32",
            "shell32",
            "advapi32",
            "user32",
            "gdi32",
        ] {
            println!("cargo:rustc-link-lib={lib}");
        }
        println!("cargo:rustc-link-arg=/DEFAULTLIB:msvcrt");
        println!("cargo:rustc-link-arg=/DEFAULTLIB:ucrt");
        println!("cargo:rustc-link-arg=/NODEFAULTLIB:libcmt");
        println!("cargo:rustc-link-arg=/NODEFAULTLIB:libucrt");
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
        let lua_dll = deps_prefix.join("bin").join("lua51.dll");
        if lua_dll.exists()
            && let Ok(out_dir) = env::var("OUT_DIR")
            && let Some(profile_dir) = PathBuf::from(out_dir).ancestors().nth(3).map(PathBuf::from)
        {
            let _ = std::fs::copy(&lua_dll, profile_dir.join("lua51.dll"));
            let _ = std::fs::copy(&lua_dll, profile_dir.join("deps").join("lua51.dll"));
        }
    } else if target_os != "windows" {
        println!("cargo:rustc-link-arg-bin=nvim=-rdynamic");
        println!("cargo:rustc-link-search=native={}", deps_lib.display());
        for (archive, lib) in [
            ("libluv.a", "static=luv"),
            ("libtree-sitter.a", "static=tree-sitter"),
            ("libutf8proc.a", "static=utf8proc"),
            ("libunibilium.a", "static=unibilium"),
            ("libluajit-5.1.a", "static=luajit-5.1"),
            ("libuv.a", "static=uv"),
        ] {
            if deps_lib.join(archive).exists() {
                println!("cargo:rustc-link-lib={lib}");
            }
        }
        for syslib in ["m", "dl", "pthread"] {
            println!("cargo:rustc-link-lib={syslib}");
        }
        if target_os == "linux" {
            println!("cargo:rustc-link-lib=util");
        } else if target_os == "macos" {
            println!("cargo:rustc-link-lib=iconv");
            println!("cargo:rustc-link-arg-bin=nvim=-Wl,-export_dynamic");
            println!("cargo:rustc-link-lib=framework=CoreServices");
        }
    }
}
