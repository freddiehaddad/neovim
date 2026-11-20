<div align="center">
  <img src="contrib/nero-logo.svg" width="360" alt="Nero">
  <h3>The Neovim you know, rebuilt in Rust.</h3>
</div>

---

**Nero** is [Neovim](https://github.com/neovim/neovim) with its entire C codebase and CMake build system replaced by Rust and Cargo.

It is a drop-in replacement for upstream Neovim, not a from-scratch rewrite with missing features. Your `init.lua`, keymaps, colorschemes, LSP servers, and plugins work without changes.

### What stays the same

- **Full plugin compatibility**: If a Lua or Vimscript plugin works in Neovim, it works here too—unmodified—on the same embedded LuaJIT runtime and editor API.
- **Built-in LSP, Treesitter, and diagnostics**: Ships with the same `runtime/` standard library (`vim.lsp`, `vim.treesitter`, `vim.diagnostic`) and tracks upstream `neovim/neovim` continuously.
- **External GUIs and RPC clients**: Speaks the exact same Msgpack-RPC protocol, so graphical frontends, language hosts, and remote plugins connect without changes.
- **Every Vim feature you rely on**: Macros, registers, marks, folds, diff mode, spell checking, quickfix, terminal buffers, and `:help`.

### What changed under the hood

- **Memory-safe Rust core with zero `.c` or `.h` files**: The entire editor core (`src/nvim/`) plus formerly vendored C libraries (`libvterm`, `libtermkey`, `unibilium`, `libmpack`, `lua-cjson`, `LPeg`, `xdiff`, `xxd`, `tee`) are native Rust (`edition = "2024"`), with `unsafe` confined strictly to C-ABI boundaries.
- **Multi-core parallelism where C is single-threaded**: Heavy workloads—including buffer-wide search (`:vimgrep`, `:global`), fuzzy file and symbol matching (`matchfuzzy`), side-by-side diff alignment (`linematch`), spell suggestions, and swap serialization—scale automatically across CPU cores.
- **Faster in-memory data structures**: Replaces C linked lists, linear scans, and 4KB pointer-paged line tables with contiguous Rust vectors, chunked line storage, and fast hash tables—speeding up soft-wrapping, virtual-text motion, option updates, and autocommand handling.
- **Pre-compiled built-in Lua runtime**: Core `runtime/lua/` modules are compiled to LuaJIT bytecode at build time, reducing startup and module-load overhead.
- **Single-command Cargo build**: No CMake, Ninja, or Luarocks bootstrap. `cargo xtask install` builds and installs the editor in one step, validated against the full functional, legacy Vim, Rust unit, and Miri lifetime test suites across Linux, macOS, and Windows.

## Performance

Measured head-to-head against release builds of [`neovim/neovim@d9a842e291`](https://github.com/neovim/neovim/commit/d9a842e291) on the same hardware across **Linux** (GCC vs. `rustc`) and **Windows** (MSVC vs. `rustc`), reporting the median of 3 interleaved runs:

<p align="center">
  <img src="contrib/benchmarks.svg" width="100%" alt="Performance comparison between Nero and Neovim on Linux and Windows">
</p>

<details>
<summary><strong>Benchmark methodology</strong></summary>

- **Speedup**: Relative speedup of Nero over Neovim (`Neovim time ÷ Nero time`; values above `1.00x` mean Nero is faster, and below `1.00x` mean Neovim is faster).
- **In-process timing**: Editor operations are timed inside the running editor with high-resolution timers (`vim.uv.hrtime()` / `reltime()`) so test-runner startup is never mixed into workload measurements.
- **Same commit & hardware**: Both builds are compiled from the same synced commit (`MAINTAIN.md`) on the same machine and filesystem (`ext4` on Linux; NTFS on Windows).
- **Release optimization**: Neovim is built with `-DCMAKE_BUILD_TYPE=Release` (`-O3 -DNDEBUG` on GCC; `/O2 /Ob2 /DNDEBUG` on MSVC), and Nero is built with `cargo xtask build --release` (`opt-level = 3`, `lto = "thin"`).
- **Reproducible runner**: Run `cargo xtask benchmark --compare --update-readme` to execute 3 interleaved rounds across both platforms and regenerate `contrib/benchmarks.svg`.

</details>

## Quick Start

### Install a pre-built release

Pre-built archives and installers for Windows (`.zip`, `.msi`), macOS (`.tar.gz`), and Linux (`.tar.gz`, `.appimage`) are published on the [Releases](https://github.com/freddiehaddad/nero/releases) page. See [INSTALL.md](./INSTALL.md) for platform instructions.

### Build and install from source

With Rust (`1.88+`) and a system C compiler installed (used once to build the bundled `libuv`, `LuaJIT`, `luv`, and `tree-sitter` archives in `.deps/`), building and installing is a single command:

```bash
cargo xtask install
```

To install to a custom prefix:

```bash
cargo xtask install --prefix /custom/path
```

Common `cargo xtask` commands (see [BUILD.md](./BUILD.md) for full details):

```bash
cargo xtask build --release   # build optimized binaries and stage runtime in target/nvim-build/
cargo xtask dist              # build release archives (.tar.gz / .zip / .appimage / .msi)
cargo xtask unittest          # run Rust unit tests
cargo xtask functionaltest    # run the Lua functional test suite
cargo xtask oldtest           # run the legacy Vim test suite
cargo xtask benchmark         # run the performance benchmark suite
cargo xtask lint              # run clippy, stylua, error-code, and vimdoc linters
```

## Project Layout

```text
├─ runtime/            Built-in Lua modules (vim.lsp, vim.treesitter, ...), syntax, and :help docs
├─ src/
│  ├─ nvim/            Neovim core in Rust
│  │  ├─ api/          Nvim API & RPC endpoints (nvim_buf_*, nvim_win_*, nvim_extmark_*, ...)
│  │  ├─ eval/         Vimscript evaluator & built-in functions
│  │  ├─ event/        Async event loop & process/pty management (libuv)
│  │  ├─ lua/          LuaJIT runtime bridge & Treesitter C API bindings
│  │  ├─ msgpack_rpc/  Msgpack-RPC channel, packer, and unpacker
│  │  ├─ os/           Platform filesystem, environment, signal, and shell abstractions
│  │  ├─ tui/          Terminal UI renderer, termkey input driver, and terminfo parser
│  │  ├─ viml/         Vimscript parser & expression lexer
│  │  └─ vterm/        Rust port of libvterm
│  ├─ cjson/           Rust port of lua-cjson
│  ├─ lpeg/            Rust port of LPeg
│  ├─ mpack/           Rust port of libmpack / lmpack
│  ├─ xdiff/           Rust port of git xdiff
│  ├─ gen/             Documentation & metadata generators
│  ├─ tee/             Rust tee helper binary
│  ├─ xxd/             Rust xxd hex dump utility
│  └─ xtask/           Cargo build, test, benchmark, lint, and packaging runner
└─ test/
   ├─ functional/      Lua functional & RPC test suites
   ├─ old/             Legacy Vimscript test suites
   ├─ benchmark/       Performance benchmark suites
   └─ event-lifetimes/ Miri callback & handle lifetime test crate
```

## Contributing & Maintenance

- [PORTING.md](./PORTING.md): Porting strategy, pitfalls encountered, and lessons learned.
- [BUILD.md](./BUILD.md): Prerequisites, platform build instructions, and `cargo xtask` reference.
- [CONTRIBUTING.md](./CONTRIBUTING.md): Workflow, testing, linting, and pull request guidelines.
- [MAINTAIN.md](./MAINTAIN.md): Upstream `neovim/neovim` sync process and release packaging.
- [AGENTS.md](./AGENTS.md): Architecture, soundness invariants, and rules for AI coding agents.

## License

Nero is licensed under the same terms as Neovim: Apache 2.0, except for portions contributed under the Vim license. Third-party components may have separate license terms; see [LICENSE.txt](./LICENSE.txt) and the license files accompanying those components.

