<div align="center">
  <img src="https://raw.githubusercontent.com/neovim/neovim.github.io/master/static/logos/neovim-logo-300x87.png" alt="Neovim">
  <h3>The editor you know, rebuilt in Rust.</h3>

  [![Test](https://github.com/freddiehaddad/neovim/actions/workflows/test.yml/badge.svg)](https://github.com/freddiehaddad/neovim/actions/workflows/test.yml)
  [![CodeQL](https://github.com/freddiehaddad/neovim/actions/workflows/codeql.yml/badge.svg)](https://github.com/freddiehaddad/neovim/actions/workflows/codeql.yml)
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

Measured head-to-head against upstream [`neovim/neovim@61e71ccf8a`](https://github.com/neovim/neovim/commit/61e71ccf8a) (`v0.13.0-dev`) on identical hardware using the built-in benchmark suite (`cargo xtask benchmark --release`):

<p align="center">
  <img src=".github/benchmarks.svg" width="100%" alt="Performance comparison between the Rust port and upstream C Neovim">
</p>

<details>
<summary><strong>View full benchmark table</strong></summary>

| Workload | Suite | Upstream C | Rust Port | Speedup |
|---|---|---:|---:|---:|
| Cold startup (headless median) | Startup | 31.4 ms | 16.8 ms | 1.87x |
| Screen layout & wrapping (24 tests) | screenpos | 36.8 s | 20.5 s | 1.80x |
| Long ASCII line layout (median) | screenpos | 55.2 µs | 24.2 µs | 2.28x |
| Long UTF-8 line layout (median) | screenpos | 167.0 µs | 79.4 µs | 2.10x |
| Regex search (backtracking) | regexp | 6.40 ms | 3.75 ms | 1.71x |
| Regex search (auto engine) | regexp | 7.94 ms | 5.96 ms | 1.33x |
| Cursor motion over 2k virtual lines | extmark | 117 µs | 72 µs | 1.63x |
| Treesitter fold navigation (100 macros) | treesitter | 60.9 ms | 42.0 ms | 1.45x |
| Treesitter redraw suite (8 tests) | treesitter | 1,535 ms | 1,507 ms | 1.02x |
| Autocmd lifecycle suite (10k events) | autocmd | 464 ms | 343 ms | 1.35x |
| Delete 10k autocmds (unique patterns) | autocmd | 27.1 ms | 17.7 ms | 1.53x |
| Extmark & virtual line suite | extmark | 197 ms | 173 ms | 1.14x |
| Option updates (140k calls) | option | 609 ms | 536 ms | 1.14x |
| Full-screen Treesitter redraw (100x50) | decor | 16.6 ms | 16.0 ms | 1.04x |

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

