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
  `cargo xtask build` compiles the application binaries (`nvim`, `tee`, `xxd`) and test fixtures, and stages a runnable tree (including generated helptags and syntax files) in `target/nvim-build/`.

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
| `cargo xtask unittest [--release]` | Run Rust library unit tests (`cargo test --lib`) |
| `cargo xtask functionaltest [args...]` | Run the Lua functional test suite (or a single `_spec.lua` file) |
| `cargo xtask oldtest [args...]` | Run the legacy Vim test suite (or a single `test_<name>.vim`) |
| `cargo xtask benchmark [--release]` | Run the performance benchmark suite in `test/benchmark/` |
| `cargo xtask clean` / `distclean` | Remove `target/` (`distclean` also removes cached `.deps/`) |

## Running Tests

```bash
# Rust unit tests
cargo xtask unittest

# Full functional test suite or a single spec file (add --release to test the optimized binary)
cargo xtask functionaltest
cargo xtask functionaltest --release test/functional/api/version_spec.lua

# Legacy Vim test suite or a single test
cargo xtask oldtest
cargo xtask oldtest --release test_syntax.vim

# Performance benchmarks (Rust --release or custom upstream C binary)
cargo xtask benchmark --release
cargo xtask benchmark --nvim-bin /path/to/upstream/build/bin/nvim
```

### Event ownership and reentrancy

The `event-lifetimes-tests` package compiles the production callback-registration, read-buffer, inbox, and timer-ownership modules without the editor's C dependencies:

```bash
cargo test -p event-lifetimes-tests
rustup toolchain install nightly-2026-11-16 --profile minimal --component miri --component rust-src
cargo +nightly-2026-11-16 miri test -p event-lifetimes-tests
```

CI repeats the Miri command with `MIRIFLAGS=-Zmiri-tree-borrows` to check both borrowing models. Miri checks Rust-side memory behavior, not libuv or Lua. Keep the native functional and legacy suites, including release builds on Windows and Linux. When using WSL, run those suites from a native Linux filesystem: tests create files and Unix sockets in and around the checkout, so moving only the Cargo target directory is insufficient. Use a separate dependency/staging tree for each platform; copied Windows parser DLLs can be selected before Linux parsers and fail to load.

Match the CI test environment when comparing results. On Windows, Node-provider fixtures need `require('neovim')` to resolve from the test directory (CI uses `npm link neovim`), and timezone-sensitive cases can be run with `TZ=UTC`. The DOS 8.3 filesystem cases invoke `fsutil` and require suitable volume settings/permissions. Record baseline prerequisite failures rather than weakening assertions or silently changing shared-machine settings.

Blocking test-client RPC requests use a runner-side deadline (`TEST_TIMEOUT`, in seconds; default 600). Individual hang-sensitive cases can set their session's `request_timeout` in milliseconds. Late responses must not interrupt subsequent requests.

For before/after process and channel measurements, preserve the baseline binary and run the same workload with each binary on the same host. Build the current test fixtures first; `--nvim-bin` changes the editor under test, while fixture executables are resolved from `NVIM_TEST_BUILD_DIR`:

```bash
cargo xtask benchmark --nvim-bin <baseline-binary> test/benchmark/event_spec.lua
cargo xtask benchmark --release test/benchmark/event_spec.lua
```

The workload measures synchronous process waits, persistent RPC round trips, and stream draining. Repeat and interleave runs; do not treat unrelated startup or regex benchmarks as evidence about event-path overhead. `--nvim-bin` also works with `functionaltest` for behavioral comparisons.

For timer ownership changes, use `test/benchmark/timer_spec.lua` with the same commands. It measures timer creation/cancellation, zero-delay callback dispatch, and Vimscript `wait()` timer cleanup over 100,000 operations, with warmups and native/deferred close processing. These are targeted lifecycle workloads, not measurements of interactive editor latency.

## Build Options

- **Debug vs. Release**: `cargo build` and `cargo xtask build` default to the `dev` profile for fast incremental builds. Pass `--release` for full optimization (`opt-level = 3`, with `lto = "thin"` configured in `Cargo.toml`). `cargo xtask install` and `cargo xtask dist` build `--release` by default.
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

