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

### Functional test sharding

`TEST_SHARD=N/M` selects one of `M` disjoint partitions of the sorted functional test groups, using one-based shard numbers. For example, set `TEST_SHARD=1/3` and run `cargo xtask functionaltest --release`; repeat with `2/3` and `3/3` to cover the full suite. Without the variable, the command still runs every group.

Sharding requires the full functional suite: do not combine it with explicit test paths, `TEST_FILE`, or benchmark runs. Invalid indices and configurations with empty shards fail rather than silently passing without coverage. For focused debugging, unset `TEST_SHARD` and select the desired test path; the test runner still fails groups in which a filter matches no tests.

Run `cargo test -p xtask` when changing test discovery or partitioning. The tests verify that the repository's complete group list is covered exactly once across all shards.

Windows CI runs the three functional shards on separate runners to bound cumulative process/console churn associated with the upstream `0xC0000142` initialization cascade. No tests are retried or removed. Each job logs its selected groups and process-resource counters; DLL-initialization failures remain failures. This isolates the runner condition rather than claiming to repair an identified Neovim memory defect.

### Event ownership and reentrancy

The `event-lifetimes-tests` package compiles the production callback-registration/shared-storage, read-buffer, inbox, and timer-ownership modules without the editor's C dependencies. Its signal and directory integration tests import the actual production adapters and supply deterministic native drivers. The shared storage primitive is used by timers, listeners, and signals; real native behavior remains covered separately by native unit and functional tests:

```bash
cargo test -p event-lifetimes-tests
rustup toolchain install nightly-2026-11-16 --profile minimal --component miri --component rust-src
cargo +nightly-2026-11-16 miri test -p event-lifetimes-tests
```

CI repeats the Miri command with `MIRIFLAGS=-Zmiri-tree-borrows` to check both borrowing models. Miri checks Rust-side memory behavior, including signal lifecycle callbacks and directory request cleanup on movement, early drop, initialization failure, and iteration errors. It does not execute libuv or establish Lua, filesystem, or OS signal semantics. Keep the native functional and legacy suites, including release builds on Windows and Linux. When using WSL, run those suites from a native Linux filesystem: tests create files and Unix sockets in and around the checkout, so moving only the Cargo target directory is insufficient. Use a separate dependency/staging tree for each platform; copied Windows parser DLLs can be selected before Linux parsers and fail to load.

Match the CI test environment when comparing results. On Windows, Node-provider fixtures need `require('neovim')` to resolve from the test directory (CI uses `npm link neovim`; a process-local `NODE_PATH` can expose an existing installation), and timezone-sensitive cases can be run with `TZ=UTC`. The DOS 8.3 filesystem cases invoke `fsutil` and require suitable volume settings/permissions. Record baseline prerequisite failures rather than weakening assertions or silently changing shared-machine settings.

The Unix filesystem unit tests inspect `/dev/stderr`. Redirecting stderr to a regular log file changes the object under test; preserve terminal/pipe semantics when capturing output, for example with a test-owned PTY.

POSIX release CI sets `NVIM_TEST_INTEG=1` to enable the live-network cases in the net and package-manager suites. Default local runs and Windows CI skip those cases; enable the flag when matching POSIX release CI coverage and distinguish gated skips from passing tests.

Blocking test-client RPC requests use a runner-side deadline (`TEST_TIMEOUT`, in seconds; default 600). Individual hang-sensitive cases can set their session's `request_timeout` in milliseconds. TCP test clients also wait for connection completion before starting RPC, using `TEST_TIMEOUT` to bound the connect wait and closing the socket on failure. Late responses or connection callbacks must not interrupt subsequent requests.

For before/after process and channel measurements, preserve the baseline binary and run the same workload with each binary on the same host. Build the current test fixtures first; `--nvim-bin` changes the editor under test, while fixture executables are resolved from `NVIM_TEST_BUILD_DIR`:

```bash
cargo xtask benchmark --nvim-bin <baseline-binary> test/benchmark/event_spec.lua
cargo xtask benchmark --release test/benchmark/event_spec.lua
```

The workload measures synchronous process waits, persistent RPC round trips, and stream draining. Repeat and interleave runs; do not treat unrelated startup or regex benchmarks as evidence about event-path overhead. `--nvim-bin` also works with `functionaltest` for behavioral comparisons.

For functional comparisons, preserve the executable basename (`nvim` / `nvim.exe`) in separate baseline/current directories. Renaming the binary changes some CLI diagnostics and TUI titles, creating unrelated expectation failures.

For timer ownership changes, use `test/benchmark/timer_spec.lua` with the same commands. It measures timer creation/cancellation, zero-delay callback dispatch, and Vimscript `wait()` timer cleanup over 100,000 operations, with warmups and native/deferred close processing. These are targeted lifecycle workloads, not measurements of interactive editor latency.

For listener ownership changes, use `test/benchmark/socket_spec.lua`. It measures 5,000 TCP listener create/close operations, draining native closes in bounded batches, plus 1,000 TCP and 1,000 pipe/Unix-socket connection/RPC/close operations. Connection workloads include test-client and RPC overhead and wait for server-side channel cleanup; they are end-to-end lifecycle measurements, not isolated allocation timings. Compare identical scripts with preserved baseline and changed binaries.

For signal ownership changes, `test/benchmark/signal_spec.lua` measures 10,000 acknowledged POSIX signal deliveries plus autocommand dispatch, after warmup. It signals only the test-owned editor and waits for each delivery, rather than assuming signal bursts cannot coalesce; it is not a raw syscall benchmark and is skipped on Windows. Also compare startup and the event/timer/socket workloads to detect broader registration-bookkeeping costs.

### API value representation

`Object` is a Rust enum; its layout is internal, while Lua/Vimscript conversion rules and Msgpack bytes remain compatibility contracts. For changes to its representation or consumers, start with:

```bash
cargo test --release --lib msgpack_rpc::packer::tests
cargo xtask functionaltest --release test/functional/api/object_spec.lua test/functional/lua/option_and_var_spec.lua test/functional/autocmd/optionset_spec.lua
cargo xtask benchmark --release test/benchmark/object_spec.lua
```

The Rust cases cover wire kinds, full-width handle decoding, binary strings, and nested/empty containers. The functional cases cover real Lua-reference copying/transfer and API-specific coercions; also run affected API, option, UI, and terminal suites.

The Object benchmark measures 100,000 nested Lua API copies, 50,000 Vimscript round trips, and 5,000 nested-payload RPC round trips after warmup. RPC uses a persistent test-owned child and includes client/transport costs. These are value-conversion workloads, not isolated enum-dispatch timings. Preserve baseline/current executable names, interleave repeated release runs on each platform without concurrent heavy validation, and report variability rather than treating layout size as a performance result.

### Directory scans and Vimscript values

Directory scans have unique RAII owners and borrow each entry from the current scan. `TvValue` is the authoritative Vimscript value enum; lock metadata and logical reference counts remain separate. Focused checks for these contracts are:

```bash
cargo test -p event-lifetimes-tests --test directories
cargo test --lib os::fs::tests::directory_scan
cargo test --lib eval::
cargo xtask functionaltest --release test/functional/vimscript/readdir_spec.lua test/functional/vimscript/typval_spec.lua test/functional/vimscript/null_spec.lua test/functional/api/object_spec.lua
cargo xtask benchmark --release test/benchmark/typval_spec.lua
cargo xtask benchmark --release test/benchmark/directory_spec.lua
```

The functional cases preserve nested directory filtering, failed-open diagnostics, scalar/container kinds, function-reference retention, temporary `v:` scopes, and lock behavior. Run the broader affected functional and legacy suites on both platforms as well.

The TypVal workload measures 100,000 Vimscript scalar-loop iterations and 10,000 mixed nested deep copies after warmup. Compare the same scripts with preserved baseline/current release binaries using `--nvim-bin`, and also check the existing Object conversion workload. Interleave repeated runs without concurrent heavy validation; enum layout or a reduced count of tags is not a performance result.

The directory workload performs 2,000 `readdir()` calls on the source directory after warmup. It includes enumeration, sorting, and Vimscript/Lua value conversion rather than isolating the native syscall or request allocation.

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

