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

Syncing with Upstream (`neovim/neovim`)
---------------------------------------

This repository tracks upstream [`neovim/neovim`](https://github.com/neovim/neovim) to stay 100% feature-, runtime-, and bugfix-compatible.

* **Last synced upstream commit**: `8bf18972c5` (`master`)

### 1. Inspecting new upstream commits

Ensure the `upstream` remote points to `https://github.com/neovim/neovim.git`:

```bash
git remote add upstream https://github.com/neovim/neovim.git 2>/dev/null || true
git fetch upstream master
git log --oneline 8bf18972c5..upstream/master
```

### 2. Mapping upstream files to this repository

Because the codebase was ported module-for-module and function-for-function, upstream changes map 1:1:

| Upstream path (`neovim/neovim`) | Path in this repository | How to port |
|---|---|---|
| `src/nvim/<path>.c`, `src/nvim/<path>.h`, `src/nvim/<path>_defs.h` | `src/nvim/<path>.rs` | Locate the matching function/struct by name in `src/nvim/<path>.rs` and port the logic to idiomatic Rust. |
| `src/nvim/version.c` (`patches_*` arrays) | `src/nvim/version.rs` (`PATCHES_*` slices & `NUM_PATCHES`) | Add newly merged Vim patch numbers to the matching `PATCHES_*` slice and increment `NUM_PATCHES`. |
| `cmake.deps/deps.txt` | `deps.txt` | Update the corresponding `*_URL` and `*_SHA256` entries in `deps.txt`. |
| `runtime/**` | `runtime/**` | Apply changes directly (`git show <sha> -- runtime/`). |
| `test/functional/**`, `test/old/testdir/**` | `test/functional/**`, `test/old/testdir/**` | Apply test additions/updates directly. |
| `test/unit/**` (LuaJIT FFI C unit tests) | `#[cfg(test)] mod tests` in `src/nvim/<module>.rs` | Port unit test assertions to the module's Rust `#[cfg(test)]` block. |
| `CMakeLists.txt`, `cmake/**`, `build.zig*` | `Cargo.toml`, `build.rs`, `src/xtask/main.rs` | Skip CMake/Zig-only changes unless they alter build definitions, version metadata, or generated runtime steps. |

### 3. Validation & recording the sync

After porting upstream commits:

1. Run `cargo xtask lintrust` (`cargo clippy --workspace --all-targets -- -D warnings`).
2. Run `cargo test` to verify all Rust unit tests pass.
3. Run the affected functional and legacy tests:
   ```bash
   cargo xtask functionaltest <path/to/spec.lua>
   cargo xtask oldtest <test_name>
   ```
4. If `:help` documentation was modified, run `cargo xtask lintdoc`.
5. Update the **Last synced upstream commit** SHA in this section of `MAINTAIN.md` and commit.

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
