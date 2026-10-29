# Building from Source

Building the Rust port of Neovim requires a current Rust toolchain and a C compiler for the four external C libraries (`libuv`, `LuaJIT`, `luv`, and `tree-sitter`) that are fetched and built automatically on first run. No CMake, Ninja, or Luarocks setup is needed.

## Quick Start

```bash
git clone https://github.com/freddiehaddad/neovim
cd neovim
cargo xtask install
```

- **Default install prefix**: `~/.local` on Linux/macOS/BSD and `%USERPROFILE%\.local\bin\nvim-win64` on Windows.
- **Custom install prefix**:
  ```bash
  cargo xtask install --prefix /custom/path
  ```
- **Build without installing**:
  ```bash
  cargo xtask build --release
  ./target/nvim-build/bin/nvim
  ```
  `cargo xtask build` compiles the workspace binaries (`nvim`, `nvim-tee`, `xxd`) and stages a complete runnable tree (including generated helptags and syntax files) in `target/nvim-build/`.

## `cargo xtask` Commands

| Command | Description |
|---|---|
| `cargo xtask build [--release]` | Build binaries and stage `runtime/` in `target/nvim-build/` |
| `cargo xtask install [--prefix <dir>]` | Build `--release` and install Neovim, runtime files, and manpages |
| `cargo xtask dist [--out-dir <dir>]` | Package release archives (`.tar.gz`/`.zip`, plus `.appimage` on Linux and `.msi` on Windows) |
| `cargo xtask deps [--force]` | Download and compile bundled C libraries (`libuv`, `LuaJIT`, `luv`, `tree-sitter`) |
| `cargo xtask doc` | Regenerate `:help` documentation and Lua `_meta` type definitions |
| `cargo xtask lint` | Run all linters (`lintrust`, `lintlua`, `linterrcodes`, `lintdoc`) |
| `cargo xtask format` | Format Rust (`cargo fmt`) and Lua (`stylua`) source files |
| `cargo xtask unittest` | Run Rust unit tests (`cargo test`) |
| `cargo xtask functionaltest [args...]` | Run the Lua functional test suite (or a single `_spec.lua` file) |
| `cargo xtask oldtest [args...]` | Run the legacy Vim test suite (or a single `test_<name>`) |
| `cargo xtask benchmark [--release]` | Run the performance benchmark suite in `test/benchmark/` |
| `cargo xtask clean` / `distclean` | Remove `target/` (`distclean` also removes cached `.deps/`) |

## Running Tests

```bash
# Rust unit tests
cargo test

# Full functional test suite or a single spec file (add --release to test the optimized binary)
cargo xtask functionaltest
cargo xtask functionaltest --release test/functional/api/version_spec.lua

# Legacy Vim test suite or a single test
cargo xtask oldtest
cargo xtask oldtest --release test_syntax

# Performance benchmarks
cargo xtask benchmark --release
```

## Build Options

- **Debug vs. Release**: `cargo build` and `cargo xtask build` default to the `dev` profile for fast incremental builds. Pass `--release` for full optimization (`opt-level = 3`, `lto = "thin"`, `codegen-units = 1`). `cargo xtask install` and `cargo xtask dist` build `--release` by default.
- **PUC Lua 5.1 instead of LuaJIT**:
  ```bash
  cargo xtask build --prefer-lua
  ```
- **System libraries instead of bundled `.deps`**:
  ```bash
  USE_BUNDLED=0 cargo build
  ```

## Platform Prerequisites

### Windows

1. Install [Rust](https://rustup.rs/) (`x86_64-pc-windows-msvc` or `aarch64-pc-windows-msvc`).
2. Install [Visual Studio](https://visualstudio.microsoft.com/downloads/) or Build Tools for Visual Studio with the **Desktop development with C++** workload.
   - You do **not** need to open a Developer Command Prompt; `build.rs` locates `cl.exe` and `vcvars64.bat` automatically via `vswhere.exe`.
3. Build or install from PowerShell or Command Prompt:
   ```pwsh
   cargo xtask install
   ```
4. *(Optional)* To package the `.msi` installer with `cargo xtask dist`, install the WiX CLI tool:
   ```pwsh
   dotnet tool install --global wix --version 5.0.2
   cargo xtask dist
   ```

### Linux

Install Rust via [rustup](https://rustup.rs/) and your distribution's C toolchain + `gettext`:

- **Ubuntu / Debian**: `sudo apt-get install build-essential curl gettext git`
- **Fedora / RHEL**: `sudo dnf -y install gcc make gettext curl git`
- **Arch Linux**: `sudo pacman -S base-devel curl gettext git`
- **Alpine Linux**: `apk add build-base coreutils curl gettext-tiny-dev git`

### macOS

```bash
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### FreeBSD / OpenBSD

- **FreeBSD**: `sudo pkg install rust gmake curl gettext git`
- **OpenBSD**: `doas pkg_add rust gmake curl gettext-tools git`

