<div align="center">
  <img src="https://raw.githubusercontent.com/neovim/neovim.github.io/master/static/logos/neovim-logo-300x87.png" alt="Neovim">
  <h3>The editor you know, rebuilt in Rust.</h3>

  [![Test](https://github.com/freddiehaddad/neovim/actions/workflows/test.yml/badge.svg)](https://github.com/freddiehaddad/neovim/actions/workflows/test.yml)
  [![CodeQL](https://github.com/freddiehaddad/neovim/actions/workflows/codeql.yml/badge.svg)](https://github.com/freddiehaddad/neovim/actions/workflows/codeql.yml)
  [![Downloads](https://img.shields.io/github/downloads/freddiehaddad/neovim/total.svg?maxAge=2592001)](https://github.com/freddiehaddad/neovim/releases/)
</div>

---

This is [Neovim](https://github.com/neovim/neovim) with its entire C codebase and CMake build system replaced by Rust and Cargo.

It is a drop-in replacement for upstream Neovim—not a from-scratch clone with missing features. Your `init.lua`, keymaps, colorschemes, LSP servers, and plugins work without changes.

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

Measured head-to-head against upstream `neovim/neovim` Nightly (`v0.13.0-dev`) on identical hardware using the built-in benchmark suite (`cargo xtask benchmark --release`):

<p align="center">
  <img src=".github/benchmarks.svg" width="100%" alt="Performance comparison between the Rust port and upstream C Neovim">
</p>

<details>
<summary><strong>View detailed benchmark numbers</strong></summary>

| Workload | Benchmark | Upstream C (`nightly`) | Rust Port (`--release`) | Result |
|---|---|---:|---:|---|
| **Cold startup** | `nvim --clean --headless +q` (median) | `31.4 ms` | **`16.7 ms`** | **1.88× faster** (`-47%`) |
| **Screen layout & UTF-8 wrapping** | `screenpos_spec.lua` (24-test suite) | `36.8 s` | **`19.9 s`** | **1.85× faster** (`-46%`) |
| — Long ASCII line `screenpos()` | `nobreakindent` / `breakindent` | `55.2 µs` / `81.3 µs` | **`23.8 µs` / `36.4 µs`** | **2.32× / 2.23× faster** |
| — Long UTF-8 line `screenpos()` | 3-byte UTF-8 (`nobreakindent` / `breakindent`) | `167.0 µs` / `201.3 µs` | **`97.1 µs` / `121.0 µs`** | **1.72× / 1.66× faster** |
| **Regex search** | `bench_regexp_spec.lua` (`regexpengine=1` / `0`) | `6.40 ms` / `7.94 ms` | **`3.48 ms` / `6.19 ms`** | **1.84× / 1.28× faster** |
| **Virtual line cursor motion** | `extmark_spec.lua` (step over 2,000 virt lines) | `117 µs` | **`71 µs`** | **1.65× faster** (`-39%`) |
| **Treesitter fold navigation** | `treesitter_spec.lua` (100 macro replays, 6k-line fold) | `60.9 ms` | **`43.1 ms`** | **1.41× faster** (`-29%`) |
| **Treesitter redraw suite** | `treesitter_spec.lua` (8-test suite total) | `1,535 ms` | **`1,433 ms`** | **1.07× faster** (`-7%`) |
| **Plugin autocmd lifecycle** | `autocmd_spec.lua` (8-test suite, 10k events) | `464 ms` | **`333 ms`** | **1.39× faster** (`-28%`) |
| — Delete 10k autocmds | Same pattern / unique patterns | `24.4 ms` / `27.1 ms` | **`13.8 ms` / `15.3 ms`** | **1.76× / 1.78× faster** |
| **Extmarks & inline diagnostics** | `extmark_spec.lua` (suite total) | `197 ms` | **`168 ms`** | **1.17× faster** (`-15%`) |
| **Option & buffer config** | `option_spec.lua` (140,000 `vim.o` / `vim.opt` updates) | `609 ms` | **`539 ms`** | **1.13× faster** (`-11%`) |
| **Full-screen Treesitter redraw** | `decor_spec.lua` (100×50 grid, median frame time) | `16.6 ms` | **`16.3 ms`** | **60+ FPS parity** |

</details>

## Quick Start

### Install a pre-built release

Pre-built archives and installers for Windows (`.zip`, `.msi`), macOS (`.tar.gz`), and Linux (`.tar.gz`, `.appimage`) are published on the [Releases](https://github.com/freddiehaddad/neovim/releases) page. See [INSTALL.md](./INSTALL.md) for platform instructions.

### Build and install from source

With Rust (`1.85+`) and a system C compiler installed (used once to build the bundled `libuv`, `LuaJIT`, `luv`, and `tree-sitter` archives in `.deps/`), building and installing is a single command:

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
├─ runtime/         Built-in Lua modules (vim.lsp, vim.treesitter, ...), syntax, and :help docs
├─ src/
│  ├─ nvim/         Neovim core in Rust
│  │  ├─ api/       Nvim API & RPC endpoints (nvim_buf_*, nvim_win_*, nvim_extmark_*, ...)
│  │  ├─ eval/      Vimscript evaluator & built-in functions
│  │  ├─ event/     Async event loop & process/pty management (libuv)
│  │  ├─ lua/       LuaJIT runtime bridge & Treesitter C API bindings
│  │  ├─ msgpack_rpc/ Msgpack-RPC channel, packer, and unpacker
│  │  ├─ os/        Platform filesystem, environment, signal, and shell abstractions
│  │  ├─ tui/       Terminal UI renderer & termkey input driver
│  │  ├─ viml/      Vimscript parser & expression lexer
│  │  └─ vterm/     Rust port of libvterm
│  ├─ cjson/        Rust port of lua-cjson
│  ├─ lpeg/         Rust port of LPeg
│  ├─ mpack/        Rust port of libmpack / lmpack
│  ├─ unibilium/    Rust port of unibilium terminfo parser
│  ├─ xdiff/        Rust port of git xdiff
│  ├─ gen/          Documentation & metadata generators
│  ├─ tee/          Rust tee helper binary
│  ├─ xxd/          Rust xxd hex dump utility
│  └─ xtask/        Cargo build, test, benchmark, lint, and packaging runner
└─ test/            Functional, benchmark, and legacy Vim test suites
```

## Contributing & Maintenance

- [BUILD.md](./BUILD.md) — Prerequisites, platform build instructions, and `cargo xtask` reference.
- [CONTRIBUTING.md](./CONTRIBUTING.md) — Workflow, testing, linting, and pull request guidelines.
- [MAINTAIN.md](./MAINTAIN.md) — Upstream `neovim/neovim` sync process and release packaging.
- [AGENTS.md](./AGENTS.md) — Architecture, soundness invariants, and rules for AI coding agents.

## License

Neovim contributions since [`b17d96`](https://github.com/neovim/neovim/commit/b17d9691a24099c9210289f16afb1a498a89d803) are licensed under the Apache 2.0 license, except for contributions ported from Vim. See [LICENSE.txt](./LICENSE.txt) for details.

