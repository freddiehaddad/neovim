<h1 align="center">
  <img src="https://raw.githubusercontent.com/neovim/neovim.github.io/master/static/logos/neovim-logo-300x87.png" alt="Neovim">

  <a href="https://neovim.io/doc/">Documentation</a>
</h1>

[![Test](https://github.com/freddiehaddad/neovim/actions/workflows/test.yml/badge.svg)](https://github.com/freddiehaddad/neovim/actions/workflows/test.yml)
[![CodeQL](https://github.com/freddiehaddad/neovim/actions/workflows/codeql.yml/badge.svg)](https://github.com/freddiehaddad/neovim/actions/workflows/codeql.yml)
[![Downloads](https://img.shields.io/github/downloads/freddiehaddad/neovim/total.svg?maxAge=2592001)](https://github.com/freddiehaddad/neovim/releases/)

This repository is a pure-Rust port of [Neovim](https://github.com/neovim/neovim), preserving full API, plugin, and runtime compatibility while replacing the legacy C codebase and CMake build system with Rust and Cargo:

- Simplify maintenance and encourage [contributions](CONTRIBUTING.md)
- Memory-safe, modular Rust architecture with zero `.c`/`.h` source files in the tree
- Enable [advanced UIs] without modifications to the core
- Maximize [extensibility](https://neovim.io/doc/user/api-ui-events.html#api-ui-events)

Features
--------

- Modern [GUIs](https://github.com/neovim/neovim/wiki/Related-projects#gui)
- [API access](https://github.com/neovim/neovim/wiki/Related-projects#api-clients)
  from any language including C/C++, C#, Clojure, D, Elixir, Go, Haskell, Java/Kotlin,
  JavaScript/Node.js, Julia, Lisp, Lua, Perl, Python, Racket, Ruby, Rust
- Embedded, scriptable [terminal emulator](https://neovim.io/doc/user/terminal.html)
- Asynchronous [job control](https://github.com/neovim/neovim/pull/2247)
- [Shared data (shada)](https://github.com/neovim/neovim/pull/2506) among multiple editor instances
- [XDG base directories](https://github.com/neovim/neovim/pull/3470) support
- Compatible with most Vim and Neovim plugins, including Lua, Ruby, and Python plugins

See [`:help nvim-features`][nvim-features] for the full list, and [`:help news`][nvim-news] for noteworthy changes in the latest version!

Install from package
--------------------

Pre-built packages for Windows, macOS, and Linux are found on the
[Releases](https://github.com/freddiehaddad/neovim/releases/) page.

See [INSTALL.md](./INSTALL.md) for detailed installation instructions.

Install from source
-------------------

See [BUILD.md](./BUILD.md) for detailed build instructions.

The build uses standard Rust `cargo` and `cargo xtask` (which automatically fetches and builds bundled third-party C libraries in `.deps/` on first run):
```bash
cargo xtask install
```

To install to a custom prefix:
```bash
cargo xtask install --prefix /full/path
```

Useful `cargo xtask` commands:

- `cargo xtask --help` lists all available build, lint, doc, package, and test subcommands.
- `cargo xtask build --release` builds the release binary and stages runtime files in `target/nvim-build/`.
- `cargo xtask dist` packages a distributable release archive (`.tar.gz` / `.zip`, plus `.appimage` on Linux and `.msi` on Windows) in `target/release/`.

Transitioning from Vim
----------------------

See [`:help nvim-from-vim`](https://neovim.io/doc/user/nvim.html#nvim-from-vim) for instructions.

Project layout
--------------

    ├─ runtime/         built-in plugins, syntax, and :help documentation
    ├─ src/
    │  ├─ cjson/        Rust port of lua-cjson
    │  ├─ gen/          Lua documentation & runtime generators
    │  ├─ lpeg/         Rust port of LPeg
    │  ├─ mpack/        Rust port of libmpack / lmpack
    │  ├─ nvim/         Neovim core Rust source code (see src/nvim/README.md)
    │  │  ├─ api/       RPC & Lua API subsystem
    │  │  ├─ eval/      Vimscript evaluation subsystem
    │  │  ├─ event/     libuv event-loop subsystem
    │  │  ├─ lua/       Lua integration & treesitter subsystem
    │  │  ├─ msgpack_rpc/ Msgpack-RPC channel & unpacker subsystem
    │  │  ├─ os/        low-level platform code
    │  │  ├─ tui/       built-in terminal UI & termkey
    │  │  ├─ viml/      Vimscript parser & expression compiler
    │  │  └─ vterm/     Rust port of libvterm
    │  ├─ tee/          Rust `tee` helper binary
    │  ├─ unibilium/    Rust port of unibilium terminfo library
    │  ├─ xdiff/        Rust port of git xdiff
    │  ├─ xtask/        Cargo build, test, lint, and packaging automation
    │  └─ xxd/          Rust `xxd` hex dump utility
    └─ test/            functional and legacy tests (see test/README.md)

License
-------

Neovim contributions since [b17d96][license-commit] are licensed under the
Apache 2.0 license, except for contributions ported from Vim. See [LICENSE.txt](./LICENSE.txt) for details.

[license-commit]: https://github.com/neovim/neovim/commit/b17d9691a24099c9210289f16afb1a498a89d803
[nvim-features]: https://neovim.io/doc/user/vim_diff.html#nvim-features
[nvim-news]: https://neovim.io/doc/user/news.html
[advanced UIs]: https://github.com/neovim/neovim/wiki/Related-projects#gui

<!-- vim: set tw=80: -->
