use std::env;
use std::path::PathBuf;
use std::process::Command;

#[path = "src/deps_build.rs"]
mod deps_build;

fn parse_nvim_metadata(cargo_toml: &str, key: &str) -> Option<String> {
    let mut in_meta = false;
    for line in cargo_toml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_meta = trimmed == "[package.metadata.nvim]";
            continue;
        }
        if in_meta
            && let Some((k, v)) = trimmed.split_once('=')
            && k.trim() == key
        {
            return Some(v.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=deps.txt");
    for env_var in [
        "DEPS_PREFIX",
        "NVIM_VERSION_MAJOR",
        "NVIM_VERSION_MINOR",
        "NVIM_VERSION_PATCH",
        "NVIM_VERSION_PRERELEASE",
        "NVIM_API_LEVEL",
        "NVIM_API_LEVEL_COMPAT",
        "NVIM_API_PRERELEASE",
        "NVIM_VERSION_LONG",
        "NVIM_VERSION_BUILD",
        "NVIM_BUILD_TYPE",
        "NVIM_VERSION_CFLAGS",
        "NVIM_DEFAULT_VIM_DIR",
        "NVIM_DEFAULT_VIMRUNTIME_DIR",
        "NVIM_DEFAULT_LIB_DIR",
        "NVIM_EXITFREE",
        "NVIM_HAVE_UNIBILIUM",
        "NVIM_HAVE_LIBINTL",
    ] {
        println!("cargo:rerun-if-env-changed={env_var}");
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let git_dir = manifest_dir.join(".git");
    if git_dir.is_dir() {
        let head_path = git_dir.join("HEAD");
        if head_path.exists() {
            println!("cargo:rerun-if-changed=.git/HEAD");
            if let Ok(head) = std::fs::read_to_string(&head_path)
                && let Some(ref_rel) = head.trim().strip_prefix("ref: ")
                && git_dir.join(ref_rel).exists()
            {
                println!("cargo:rerun-if-changed=.git/{ref_rel}");
            }
        }
        if git_dir.join("packed-refs").exists() {
            println!("cargo:rerun-if-changed=.git/packed-refs");
        }
    }
    let cargo_toml = std::fs::read_to_string(manifest_dir.join("Cargo.toml")).unwrap_or_default();

    let ver_major = env::var("NVIM_VERSION_MAJOR")
        .or_else(|_| env::var("CARGO_PKG_VERSION_MAJOR"))
        .unwrap_or_else(|_| "0".to_string());
    let ver_minor = env::var("NVIM_VERSION_MINOR")
        .or_else(|_| env::var("CARGO_PKG_VERSION_MINOR"))
        .unwrap_or_else(|_| "13".to_string());
    let ver_patch = env::var("NVIM_VERSION_PATCH")
        .or_else(|_| env::var("CARGO_PKG_VERSION_PATCH"))
        .unwrap_or_else(|_| "0".to_string());
    let pkg_pre = env::var("CARGO_PKG_VERSION_PRE").unwrap_or_default();
    let ver_prerelease = env::var("NVIM_VERSION_PRERELEASE").unwrap_or_else(|_| {
        if pkg_pre.is_empty() {
            String::new()
        } else {
            format!("-{pkg_pre}")
        }
    });
    let api_level = env::var("NVIM_API_LEVEL")
        .ok()
        .or_else(|| parse_nvim_metadata(&cargo_toml, "api_level"))
        .unwrap_or_else(|| "15".to_string());
    let api_compat = env::var("NVIM_API_LEVEL_COMPAT").unwrap_or_else(|_| "0".to_string());
    let is_api_prerelease = !ver_prerelease.is_empty() && ver_patch == "0";
    let api_prerelease =
        env::var("NVIM_API_PRERELEASE").unwrap_or_else(|_| is_api_prerelease.to_string());

    println!("cargo:rustc-env=NVIM_VERSION_MAJOR={ver_major}");
    println!("cargo:rustc-env=NVIM_VERSION_MINOR={ver_minor}");
    println!("cargo:rustc-env=NVIM_VERSION_PATCH={ver_patch}");
    println!("cargo:rustc-env=NVIM_VERSION_PRERELEASE={ver_prerelease}");
    println!("cargo:rustc-env=NVIM_API_LEVEL={api_level}");
    println!("cargo:rustc-env=NVIM_API_LEVEL_COMPAT={api_compat}");
    println!("cargo:rustc-env=NVIM_API_PRERELEASE={api_prerelease}");

    if env::var("NVIM_DEFAULT_VIM_DIR").is_err() {
        let default_vim_dir = if cfg!(windows) {
            "C:/Program Files/nvim/share/nvim"
        } else {
            "/usr/local/share/nvim"
        };
        println!("cargo:rustc-env=NVIM_DEFAULT_VIM_DIR={default_vim_dir}");
    }

    if env::var("NVIM_VERSION_LONG").is_err() {
        let base_ver = format!("v{ver_major}.{ver_minor}.{ver_patch}");
        let mut version = format!("{base_ver}{ver_prerelease}");
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
                if !ver_prerelease.is_empty() {
                    let prefix = format!("{base_ver}-");
                    let rest = tag
                        .strip_prefix(&prefix)
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
                    version = format!("{base_ver}{ver_prerelease}-{formatted}");
                }
            }
        }
        println!("cargo:rustc-env=NVIM_VERSION_LONG=NVIM {version}");
        println!("cargo:rustc-env=NVIM_VERSION_BUILD={build}");
    }

    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    if env::var("NVIM_BUILD_TYPE").is_err() {
        let bt = if profile == "release" {
            "Release"
        } else {
            "Debug"
        };
        println!("cargo:rustc-env=NVIM_BUILD_TYPE=Build type: {bt}");
    }
    if env::var("NVIM_VERSION_CFLAGS").is_err() && profile != "release" {
        let unibilium = match env::var("NVIM_HAVE_UNIBILIUM").as_deref() {
            Ok("0" | "false" | "OFF" | "off") => "0",
            _ => "1",
        };
        println!(
            "cargo:rustc-env=NVIM_VERSION_CFLAGS=Compilation: cargo -DHAVE_UNIBILIUM={unibilium}"
        );
    }

    let deps_prefix = env::var("DEPS_PREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join(".deps").join("usr"));
    let prefer_lua = env::var("CARGO_FEATURE_VENDOR_BIT").is_ok();
    deps_build::ensure_deps(&manifest_dir, &deps_prefix, prefer_lua, false);
    let deps_lib = deps_prefix.join("lib");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    if target_os == "windows" && target_env == "msvc" {
        println!("cargo:rustc-link-search=native={}", deps_lib.display());
        for lib in [
            "luv",
            "tree-sitter",
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
        println!("cargo:rustc-link-arg-bin=nvim=/STACK:8388608");
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
        let lua_entry = if prefer_lua && deps_lib.join("liblua.a").exists() {
            ("liblua.a", "static=lua", "lua5.1")
        } else {
            ("libluajit-5.1.a", "static=luajit-5.1", "luajit-5.1")
        };
        for (archive, static_lib, sys_lib) in [
            ("libluv.a", "static=luv", "luv"),
            ("libtree-sitter.a", "static=tree-sitter", "tree-sitter"),
            lua_entry,
            ("libuv.a", "static=uv", "uv"),
        ] {
            if deps_lib.join(archive).exists() {
                println!("cargo:rustc-link-lib={static_lib}");
            } else {
                println!("cargo:rustc-link-lib={sys_lib}");
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
