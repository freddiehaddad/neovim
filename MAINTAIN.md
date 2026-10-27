Maintaining the Neovim Rust Project
===================================

Notes on maintaining this repository.

General guidelines
------------------

* Decide by cost-benefit
* Write down what was decided
* Constraints are good
* Use automation (`cargo xtask` and GitHub Actions) to solve problems
* Never break the RPC/plugin API

Versioning and Releases
-----------------------

Version metadata lives in a single place at the top of `Cargo.toml`:

```toml
[package]
name = "neovim"
version = "0.13.0-dev"
edition = "2024"

[package.metadata.nvim]
api_level = 15
```

`build.rs` automatically derives `NVIM_VERSION_{MAJOR,MINOR,PATCH,PRERELEASE}`, `NVIM_API_PRERELEASE`, and `NVIM_API_LEVEL_COMPAT` from `Cargo.toml`:

1. **Cutting a release (`vX.Y.Z`)**:
   - In `Cargo.toml`, change `version = "X.Y.Z-dev"` to `version = "X.Y.Z"`.
   - If releasing a new minor/major version (`Z == 0`), generate the frozen API metadata fixture:
     ```bash
     cargo xtask build --release
     ./target/nvim-build/bin/nvim --api-info > test/functional/fixtures/api_level_<N>.mpack
     cargo xtask doc
     ```
   - Commit, tag `vX.Y.Z`, and push the tag (`release.yml` builds and publishes all platform archives, AppImages, and Windows MSI installers).
2. **Starting the next development cycle**:
   - Bump `version` in `Cargo.toml` to the next `-dev` version (and increment `api_level` if starting a new minor/major cycle).

Deprecating and removing features
---------------------------------

When a non-experimental feature is slated to be removed:

1. **Soft deprecate** in the next release (`@deprecated` annotation, note in `runtime/doc/deprecated.txt`, and `vim.deprecate()`).
2. **Hard deprecate** in the following release (issue a deprecation warning on use).
3. **Remove** in a subsequent release.

Dependencies
------------

### Bundled C dependencies (`deps.txt`)

Bundled third-party C libraries are defined in `deps.txt` and compiled by `src/deps_build.rs` into `.deps/usr`:

* [LuaJIT](https://github.com/LuaJIT/LuaJIT)
* [Lua 5.1](https://www.lua.org/download.html) (used when building with `--prefer-lua`)
* [libuv](https://github.com/libuv/libuv)
* [Luv](https://github.com/luvit/luv)
    * When bumping, also sync `runtime/lua/uv/_meta.lua` and `runtime/doc/luvref.txt`.
* [lua-compat-5.3](https://github.com/lunarmodules/lua-compat-5.3)
* [tree-sitter](https://github.com/tree-sitter/tree-sitter) and bundled parsers (`c`, `diff`, `lua`, `vim`, `vimdoc`, `query`, `markdown`, `markdown_inline`)
* [win32yank](https://github.com/equalsraf/win32yank) (Windows clipboard helper)

### In-tree Rust subsystems (ported from formerly vendored C libraries)

* `src/mpack/`: Rust port of `libmpack` and `libmpack-lua`
* `src/xdiff/`: Rust port of `git` `xdiff`
* `src/cjson/`: Rust port of `lua-cjson`
* `src/lpeg/`: Rust port of `LPeg`
* `src/unibilium/`: Rust port of `unibilium` terminfo parser
* `src/nvim/vterm/`: Rust port of `libvterm`
* `src/nvim/tui/termkey/`: Rust port of `libtermkey`
* `src/bit.rs`: Rust port of `LuaBitOp` (enabled with `--features vendor-bit` / `--prefer-lua`)

### Generated Lua & Metadata files

* `runtime/lua/vim/lsp/_meta/protocol.lua`: LSP specification (`src/gen/gen_lsp.lua`)
* `runtime/doc/*.txt` & `runtime/lua/vim/_meta/*.gen.lua`: Regenerated via `cargo xtask doc`

Automation (CI)
---------------

* CI jobs are defined in `.github/workflows/`:
    * `test.yml`: Runs `cargo fmt --check`, `cargo xtask lintrust`, `cargo xtask unittest`, `cargo xtask lintlua`, `cargo xtask linterrcodes`, `cargo xtask lintcommit`, `cargo xtask doc`, `cargo xtask lintdoc`, and full functional/legacy/install test suites across Linux, macOS, and Windows.
    * `test_bsd.yml`: Verifies FreeBSD and OpenBSD builds via `cargo xtask build`.
    * `release.yml`: Builds and publishes Linux `.tar.gz`/`.appimage`, macOS `.tar.gz`, and Windows `.zip`/`.msi` release assets via `cargo xtask dist`.
    * `codeql.yml` & `zizmor.yml`: Static security analysis for Rust code and GitHub Actions workflows.
