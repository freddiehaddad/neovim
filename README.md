<div align="center">
  <img src="https://raw.githubusercontent.com/neovim/neovim.github.io/master/static/logos/neovim-logo-300x87.png" alt="Neovim">
  <h3>The editor you know, rebuilt in Rust.</h3>

  [![Test](https://github.com/freddiehaddad/neovim/actions/workflows/test.yml/badge.svg)](https://github.com/freddiehaddad/neovim/actions/workflows/test.yml)
  [![Downloads](https://img.shields.io/github/downloads/freddiehaddad/neovim/total.svg?maxAge=2592001)](https://github.com/freddiehaddad/neovim/releases/)
</div>

---

This is [Neovim](https://github.com/neovim/neovim) with its entire C codebase and CMake build system replaced by Rust and Cargo.

It is a drop-in replacement for upstream Neovim, not a from-scratch rewrite with missing features. Your `init.lua`, keymaps, colorschemes, LSP servers, and plugins work without changes.

### What stays the same

- **All your Lua and Vimscript plugins**: `lazy.nvim`, `telescope.nvim`, `nvim-treesitter`, `blink.cmp`, `gitsigns.nvim`, `mason.nvim`, and the rest of the ecosystem run unmodified on the same embedded LuaJIT runtime.
- **Built-in LSP, Treesitter, and diagnostic engine**: Shares the exact `runtime/` Lua standard library (`vim.lsp`, `vim.treesitter`, `vim.diagnostic`) and tracks upstream `neovim/neovim` commits continuously.
- **External GUIs and RPC clients**: 100% compatible with the Neovim Msgpack-RPC protocol (`neovide`, `nvim-qt`, `fvim`, `pynvim`, `neovim-node-client`, and remote plugins).
- **Every Vim feature you rely on**: Macros, registers, marks, folds, diff mode, spell checking, quickfix, terminal buffers, and `:help`.

### What changed under the hood

- **Zero `.c` or `.h` files in the repository**: All 460K+ lines of the editor core (`src/nvim/`) plus formerly vendored C libraries (`libvterm`, `libtermkey`, `unibilium`, `libmpack`, `lua-cjson`, `LPeg`, `xdiff`, `xxd`, `tee`) are native Rust (`edition = "2024"`).
- **Standard Cargo workflow**: No CMake, Ninja, or Luarocks bootstrap. `cargo xtask install` builds and installs the editor in one step.
- **Validated against the full test suite**: Passes all 274 functional test suites, 131 legacy Vim test suites, and 1,000+ Rust unit tests across Linux, macOS, and Windows.

## Performance

Measured head-to-head against retail (`CMAKE_BUILD_TYPE=Release`) builds of upstream [`neovim/neovim@629e501cd6`](https://github.com/neovim/neovim/commit/629e501cd6) (`v0.13.0-dev`) on identical hardware across **Linux** (GCC `Release` vs. `rustc --release`) and **Windows** (MSVC `Release` vs. `rustc --release`) using 3 interleaved runs via `cargo xtask benchmark --compare`:

<p align="center">
  <img src=".github/benchmarks.svg" width="100%" alt="Performance comparison between the Rust port and upstream C Neovim on Linux and Windows">
</p>

<details>
<summary><strong>View full benchmark table &amp; methodology</strong></summary>

| Workload | Suite | Linux C | Linux Rust | Speedup | Windows C | Windows Rust | Speedup |
|---|---|---:|---:|---:|---:|---:|---:|
| Cold startup (`--clean --headless +q`, 25-run median) | Startup | 4.29 ms | 8.27 ms | 0.52x | 43.52 ms | 34.91 ms | 1.25x |
| Screen layout & wrapping (24 tests) | screenpos | 24.18 s | 16.00 s | 1.51x | 29.33 s | 17.01 s | 1.72x |
| Long ASCII line layout (median) | screenpos | 3.61 µs | 1.36 µs | 2.65x | 70.00 µs | 1.50 µs | 46.67x |
| Long UTF-8 line layout (median) | screenpos | 93.92 µs | 72.48 µs | 1.30x | 107.00 µs | 75.90 µs | 1.41x |
| Regex search (backtracking) | regexp | 2.35 ms | 5.41 ms | 0.43x | 4.57 ms | 5.62 ms | 0.81x |
| Regex search (auto engine) | regexp | 3.29 ms | 7.34 ms | 0.45x | 6.24 ms | 9.17 ms | 0.68x |
| Cursor motion over 2k virtual lines | extmark | 59.32 µs | 47.01 µs | 1.26x | 68.20 µs | 54.00 µs | 1.26x |
| Treesitter fold navigation (100 macros) | treesitter | 14.33 ms | 18.84 ms | 0.76x | 63.34 ms | 48.00 ms | 1.32x |
| Treesitter redraw suite (8 tests) | treesitter | 1153 ms | 1097 ms | 1.05x | 1638 ms | 1341 ms | 1.22x |
| Autocmd lifecycle suite (10k events) | autocmd | 129.6 ms | 136.8 ms | 0.95x | 444.9 ms | 336.3 ms | 1.32x |
| Delete 10k autocmds (unique patterns) | autocmd | 21.85 ms | 1.10 ms | 19.81x | 25.82 ms | 2.09 ms | 12.38x |
| Extmark & virtual line suite | extmark | 111.7 ms | 134.3 ms | 0.83x | 201.9 ms | 215.8 ms | 0.94x |
| Option updates (140k calls) | option | 225.1 ms | 234.6 ms | 0.96x | 560.9 ms | 431.1 ms | 1.30x |
| Full-screen Treesitter redraw (100x50) | decor | 14.00 ms | 15.50 ms | 0.90x | 15.60 ms | 16.10 ms | 0.97x |

**Methodology**:
- **Speedup**: Relative speedup of the Rust port compared to upstream C (`C time ÷ Rust time`; values `> 1.00x` mean Rust is faster, and `< 1.00x` mean C is faster).
- Both repositories are checked out at the exact same synced upstream commit (`MAINTAIN.md`) on the same filesystem (`ext4` on Linux; NTFS on Windows).
- Upstream C is built from source with `-DCMAKE_BUILD_TYPE=Release` (`-O3 -DNDEBUG` on Linux GCC; `/O2 /Ob2 /DNDEBUG` on Windows MSVC).
- The Rust port is built with `cargo xtask build --release` (`opt-level = 3`, `lto = "thin"`).
- Benchmarks are executed across 3 interleaved rounds (`cargo xtask benchmark --compare --update-readme`) and the median of the 3 rounds is reported.

</details>

## Quick Start

### Install a pre-built release

Pre-built archives and installers for Windows (`.zip`, `.msi`), macOS (`.tar.gz`), and Linux (`.tar.gz`, `.appimage`) are published on the [Releases](https://github.com/freddiehaddad/neovim/releases) page. See [INSTALL.md](./INSTALL.md) for platform instructions.

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
│  │  ├─ tui/          Terminal UI renderer & termkey input driver
│  │  ├─ viml/         Vimscript parser & expression lexer
│  │  └─ vterm/        Rust port of libvterm
│  ├─ cjson/           Rust port of lua-cjson
│  ├─ lpeg/            Rust port of LPeg
│  ├─ mpack/           Rust port of libmpack / lmpack
│  ├─ unibilium/       Rust port of unibilium terminfo parser
│  ├─ xdiff/           Rust port of git xdiff
│  ├─ gen/             Documentation & metadata generators
│  ├─ tee/             Rust tee helper binary
│  ├─ xxd/             Rust xxd hex dump utility
│  └─ xtask/           Cargo build, test, benchmark, lint, and packaging runner
└─ test/               Functional, benchmark, and legacy Vim test suites
```

## Contributing & Maintenance

- [PORTING.md](./PORTING.md): Porting strategy, pitfalls encountered, and lessons learned.
- [BUILD.md](./BUILD.md): Prerequisites, platform build instructions, and `cargo xtask` reference.
- [CONTRIBUTING.md](./CONTRIBUTING.md): Workflow, testing, linting, and pull request guidelines.
- [MAINTAIN.md](./MAINTAIN.md): Upstream `neovim/neovim` sync process and release packaging.
- [AGENTS.md](./AGENTS.md): Architecture, soundness invariants, and rules for AI coding agents.

## License

Neovim contributions since [`b17d96`](https://github.com/neovim/neovim/commit/b17d9691a24099c9210289f16afb1a498a89d803) are licensed under the Apache 2.0 license, except for contributions ported from Vim. See [LICENSE.txt](./LICENSE.txt) for details.

