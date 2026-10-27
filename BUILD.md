# Building Neovim (Rust)

- **IMPORTANT**: Before upgrading to a new version, **always check for [breaking changes](https://neovim.io/doc/user/news.html#news-breaking).**

## Quick start

1. Install [build prerequisites](#build-prerequisites) on your system.
2. Clone the repository:
   ```bash
   git clone https://github.com/freddiehaddad/neovim
   cd neovim
   ```
3. Build and install:
   ```bash
   cargo xtask install
   ```
   - Default install location is `~/.local` on Unix and `%USERPROFILE%\.local\bin\nvim-win64` on Windows.
   - To install to a custom directory:
     ```bash
     cargo xtask install --prefix /custom/path
     ```

**Notes**:
- Running `cargo build` or any `cargo xtask` command automatically downloads and builds the bundled third-party C libraries (`libuv`, `LuaJIT`, `luv`, `tree-sitter`, and bundled tree-sitter parsers) into `.deps/` on first run.
- `cargo xtask build` (or `cargo xtask build --release`) compiles the binaries and stages a complete runnable tree (including generated helptags and syntax files) in `target/nvim-build/`.
- You can run the staged binary directly without installing:
  ```bash
  ./target/nvim-build/bin/nvim
  ```

## `cargo xtask` Reference

| Command | Description |
|---|---|
| `cargo xtask deps [--force]` | Download and compile bundled C dependencies in `.deps/` |
| `cargo xtask build [--release]` | Compile binaries and stage runtime in `target/nvim-build/` |
| `cargo xtask install [--prefix <dir>]` | Build release and install Neovim, runtime, and locale `.mo` files |
| `cargo xtask dist [--out-dir <dir>]` | Package release archives (`.tar.gz`/`.zip`, plus `.appimage`/`.msi`) in `target/release/` |
| `cargo xtask doc` | Regenerate `:help` documentation files from source docstrings |
| `cargo xtask lint` | Run all linters (`lintrust`, `lintlua`, `linterrcodes`, `lintdoc`) |
| `cargo xtask unittest` | Run Rust unit tests (`cargo test`) |
| `cargo xtask functionaltest [args...]` | Run Lua functional test suite |
| `cargo xtask oldtest [args...]` | Run legacy Vim test suite |
| `cargo xtask clean` / `distclean` | Clean `target/` (`distclean` also removes `.deps/`) |

## Running tests

See [runtime/doc/dev_test.txt](./runtime/doc/dev_test.txt) for details on running and writing tests:

```bash
# Rust unit tests
cargo test

# Full functional test suite (or a single spec file)
cargo xtask functionaltest
cargo xtask functionaltest test/functional/api/version_spec.lua

# Legacy Vim tests (all or a single test)
cargo xtask oldtest
cargo xtask oldtest test_syntax
```

## Build configurations

### Debug vs. Release

- **Debug** (default for `cargo build` / `cargo xtask build`): Fast incremental compilation with debug assertions and symbols.
- **Release** (`cargo build --release` / `cargo xtask build --release`): Full optimizations (`opt-level = 3`). Used by `cargo xtask install` and `cargo xtask dist` by default (pass `--debug` to override).

### PUC Lua

To build against standard PUC Lua 5.1 instead of LuaJIT:
```bash
cargo xtask build --prefer-lua
```

### Using system libraries instead of `.deps`

By default, `build.rs` and `cargo xtask` build and statically link the bundled libraries in `.deps/usr`. To skip building `.deps` and link against system-installed `libuv`, `luajit`, `luv`, and `tree-sitter`:
```bash
USE_BUNDLED=0 cargo build
```

## Building on Windows

1. Install [Rust](https://rustup.rs/) (`x86_64-pc-windows-msvc` or `aarch64-pc-windows-msvc`).
2. Install [Visual Studio](https://visualstudio.microsoft.com/downloads/) (or Build Tools for Visual Studio) with the **Desktop development with C++** workload.
   - You do **not** need to open a special Developer Command Prompt—`build.rs` and `src/deps_build.rs` automatically locate `cl.exe` / `vcvars64.bat` via `vswhere.exe`.
3. Ensure `curl.exe` and `tar.exe` are available in `PATH` (included by default in Windows 10/11).
4. Build or install from PowerShell or Command Prompt:
   ```pwsh
   cargo xtask build --release
   cargo xtask install
   ```
5. (Optional) To build the Windows `.msi` installer with `cargo xtask dist`, install the WiX CLI tool:
   ```pwsh
   dotnet tool install --global wix --version 5.0.2
   cargo xtask dist
   ```

## Build prerequisites

### General requirements

- **Rust toolchain** (Rust 1.85+ / Edition 2024, via [rustup](https://rustup.rs/))
- A C compiler (`cc` / `gcc` / `clang` on Unix; MSVC `cl.exe` on Windows) and `make` (`gmake` on BSD) to compile the 4 bundled C dependencies (`libuv`, `LuaJIT`, `luv`, `tree-sitter`)
- `curl`, `tar`, and `git`

### Ubuntu / Debian

```bash
sudo apt-get install build-essential curl gettext git
```

### RHEL / Fedora

```bash
sudo dnf -y install gcc make gettext curl git
```

### Arch Linux

```bash
sudo pacman -S base-devel curl gettext git
```

### Alpine Linux

```bash
apk add build-base coreutils curl gettext-tiny-dev git
```

### macOS

1. Install Xcode Command Line Tools:
   ```bash
   xcode-select --install
   ```
2. Install Rust via `rustup`:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

### FreeBSD

```bash
sudo pkg install rust gmake curl gettext git
```

### OpenBSD

```bash
doas pkg_add rust gmake curl gettext-tools git
```

- Requires the [Emscripten SDK](https://emscripten.org/docs/getting_started/downloads.html) (`emsdk`).
- After installing and activating it, pass its sysroot to `-Demscripten-sysroot`, e.g. `$EMSDK/upstream/emscripten/cache/sysroot`.

To build the WebAssembly target:

```bash
zig build nvim_bin \
  -Dtarget=wasm32-emscripten \
  -Demscripten-sysroot=$EMSDK/upstream/emscripten/cache/sysroot
```

This currently produces:

- `nvim.wasm`, the WebAssembly module.
- `nvim.js`, the JavaScript loader generated by Emscripten.
