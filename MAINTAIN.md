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

* **Last synced upstream commit**: `4fb996ce73` (`master`)

### 1. Inspecting new upstream commits

Ensure the `upstream` remote points to `https://github.com/neovim/neovim.git`:

```bash
git remote add upstream https://github.com/neovim/neovim.git 2>/dev/null || true
git fetch upstream master
git log --oneline 4fb996ce73..upstream/master
```

### 2. Mapping upstream files to this repository

Module responsibilities generally follow upstream. Internal ownership and layouts intentionally differ; port behavior, not C pointer lifetimes or struct layouts:

| Upstream path (`neovim/neovim`) | Path in this repository | How to port |
|---|---|---|
| `src/nvim/<path>.c`, `src/nvim/<path>.h`, `src/nvim/<path>_defs.h` | `src/nvim/<path>.rs` | Locate the matching function/struct by name in `src/nvim/<path>.rs` and port the logic to idiomatic Rust. |
| `src/nvim/memline.c`, `src/nvim/memline_defs.h` | `src/nvim/memline.rs` | In-memory buffer lines live in `Memline::ml_chunks` (`Vec<LineChunk>`, 400–800 `Send + Sync` `LineEntry`s per chunk with `start_lnum`, `totalsize`, and cached `swap_blocks`); 4KB `PointerBlock`/`DataBlock` pages are serialized on demand in `ml_sync_swap_blocks` during `.swp` sync/preserve and read during `ml_recover`. |
| `src/nvim/regexp.c`, `src/nvim/regexp_nfa.c`, `src/nvim/regexp_bt.c` | `src/nvim/regexp.rs`, `src/nvim/regexp_nfa.rs`, `src/nvim/regexp_bt.rs` | Compiled `NfaRegprog` and `BtRegprog` are immutable (`Send + Sync`); per-run NFA execution lists (`lastlist_buf`, `rec_listbuf`) live in thread-local `RegexpNfaState` so pure single-line searches (`ml_try_mark_global_lines`, `ml_try_vimgrep_lines`) can run across worker threads via `std::thread::scope`. |
| `src/nvim/globals.h` | `src/nvim/globals.rs` (domain subsystems) or module `thread_local!` | Single-module globals live in their owning module's `thread_local!` state; cross-module globals live in one of the 9 decentralized domain subsystems (see table below). |
| `src/nvim/event/**`, process/channel code | `src/nvim/event/`, `src/nvim/channel.rs`, `src/nvim/msgpack_rpc/channel.rs` | Use shared `Proc`/`Stream` handles, `Rc<Channel>` lookup, and shared main-thread loop access. Preserve logical reference counts and event timing separately from Rust allocation lifetimes. Native callback registrations retain storage until their close callbacks finish. |
| `src/nvim/version.c` (`patches_*` arrays) | `src/nvim/version.rs` (`PATCHES_*` slices & `NUM_PATCHES`) | Add newly merged Vim patch numbers to the matching `PATCHES_*` slice and increment `NUM_PATCHES`. |
| `cmake.deps/deps.txt` | `deps.txt` | Update the corresponding `*_URL` and `*_SHA256` entries in `deps.txt`. |
| `runtime/**` | `runtime/**` | Apply changes directly (`git show <sha> -- runtime/`). |
| `test/functional/**`, `test/old/testdir/**` | `test/functional/**`, `test/old/testdir/**` | Apply test additions/updates directly. |
| `test/unit/**` (LuaJIT FFI C unit tests) | `#[cfg(test)] mod tests` in `src/nvim/<module>.rs` | Port unit test assertions to the module's Rust `#[cfg(test)]` block. |
| `CMakeLists.txt`, `cmake/**`, `build.zig*` | `Cargo.toml`, `build.rs`, `src/xtask/main.rs` | Skip CMake/Zig-only changes unless they alter build definitions, version metadata, or generated runtime steps. |

#### `src/nvim/globals.h` -> Domain Subsystem Mapping

Cross-module state from upstream `src/nvim/globals.h` is partitioned into 9 decentralized domain subsystem structs, each defined with its own `thread_local!` in its owning module and re-exported from `src/nvim/globals.rs`:

| Subsystem accessor | Struct & owning module | Upstream `globals.h` domain |
|---|---|---|
| `workspace_state()` | `WorkspaceState` (`src/nvim/globals.rs`) | Window/buffer/tabpage/frame graph (`firstwin`, `lastwin`, `prevwin`, `topframe`, `first_tabpage`, `curtab`, `lastused_tabpage`, `firstbuf`, `lastbuf`, `curbuf`), handle registries (`find_buffer`, `find_window`, `find_tabpage`), diff/scrollbind flags, working directory (`globaldir`, `last_chdir_reason`), swapfile prompts (`swap_exists_*`), and RAII guards (`disallow_tabpage_move`, `pause_fold_update`, `pause_check_timestamps`). |
| `msg_state()` | `MessageGlobalState` (`src/nvim/message.rs`) | Message grid/scrolling (`msg_col`, `msg_row`, `msg_scrolled*`, `msg_grid*`, `keep_msg*`), error/warning suppression (`emsg_off`, `emsg_skip`, `emsg_silent`, `msg_silent`, `did_emsg`, `called_emsg`, `suppress_emsg`, `skip_emsg`), hit-enter state (`need_wait_return`, `no_wait_return`, `suppress_wait_return`), and `:redir` state (`redir_*`, `capture_ga`). |
| `exc_state()` | `ExceptionGlobalState` (`src/nvim/ex_eval.rs`) | Exception and `:try` stack (`current_exception`, `did_throw`, `need_rethrow`, `trylevel`, `force_abort`, `msg_list`, `push_caught`/`pop_caught`/`peek_caught`), debugger/profiler (`debug_*`, `do_profiling`, `ex_nesting_level`), script/GC context (`current_sctx`, `may_garbage_collect`, `want_garbage_collect`), and process lifecycle (`exiting`, `v_dying`, `readonlymode`, `recoverymode`, `headless_mode`, `embedded_mode`, `g_stats`). |
| `screen_state()` | `ScreenGlobalState` (`src/nvim/drawscreen.rs`) | Screen dimensions (`rows_global`, `columns_global`), redraw flags (`must_redraw`, `skip_redraw`, `do_redraw`, `updating_screen`, `redrawing_disabled`, `disable_redrawing`, `override_redrawing_disabled`), grids and line buffers (`default_grid*`, `with_linebuf*`, `pum_grid`, `pum_want`), highlight tables (`highlight_attr*`, `normal_fg`/`bg`/`sp`, `ns_hl_*`), decorations (`with_decor_state*`, `push_win_extmark`, `with_tab_page_click_defs*`), and UI state (`current_ui`, `ui_client_*`, `resize_events`). |
| `input_state()` | `InputGlobalState` (`src/nvim/input.rs`) | Typeahead buffer (`typebuf`, `vgetc_*`, `mod_mask`, `key_typed`, `key_stuffed`, `maptick`), macro recording/execution (`reg_recording`, `reg_executing`, `exec_from_reg`), mapping/interrupt controls (`no_mapping`, `allow_keys`, `state_waiting`, `got_int`), `-W` script output (`with_scriptout_mut`, `set_scriptout`, `close_scriptout`), mouse coordinates (`mouse_grid`, `mouse_row`, `mouse_col`, `mouse_dragging`), and security locks (`sandbox`, `textlock`, `allbuf_lock`, `secure`, `expr_map_lock`, `lock_text`, `override_textlock`, `lock_allbuf`, `enter_sandbox`). |
| `cmdline_state()` | `CmdlineGlobalState` (`src/nvim/ex_getln.rs`) | Command-line UI (`cmdline_row`, `redraw_cmdline`, `redraw_mode`, `clear_cmdline`, `mode_displayed`, `cmdline_star`), Ex/command modifiers (`cmod_flags`, `cmod_split`, `cmod_tab`, `take_cmdmod`/`set_cmdmod`, `exmode_active`, `silent_mode`, `cmdpreview`, `global_busy`, `listcmd_busy`), command history (`last_cmdline`, `repeat_cmdline`, `new_last_cmdline`), wildmenu (`wild_menu_showing`, `wim_flags`), and command-line window (`cmdwin_type`, `cmdwin_buf`, `cmdline_win`). |
| `autocmd_state()` | `AutocmdGlobalState` (`src/nvim/autocmd.rs`) | Autocommand execution state (`autocmd_busy`, `autocmd_no_enter`, `autocmd_no_leave`, `block_enter`, `block_leave`, `au_new_curbuf`, `autocmd_fname*`, `autocmd_bufnr`, `autocmd_match`), deferred buf/win free queues (`defer_free_buf`, `defer_free_win`, `clear_pending_frees`), and `CursorMoved`/`CursorHold` tracking (`last_cursormoved*`, `did_cursorhold`). |
| `search_state()` | `SearchGlobalState` (`src/nvim/search.rs`) | Search engine state (`search_global`, `with_screen_search_hl*`, `search_hl_has_cursor_lnum`), `:substitute` counters (`sub_nsubs`, `sub_nlines`), and tag preview flags (`g_do_tagpreview`, `g_tag_at_cursor`). |
| `edit_state()` | `EditGlobalState` (`src/nvim/normal.rs`) | Editor mode (`state_global`, `last_mode`, `edit_submode*`), Visual/Select state (`visual_global`, `km_stopsel`, `km_startsel`, `virtual_op`), Insert/Replace state (`ins_global`, `where_paste_started`, `saved_cursor`, `old_indent`, `replace_offset`, `append_last_insert`/`with_last_insert`), Normal/Operator state (`opcount`, `motion_force`, `finish_op`, `dollar_vcol`), and undo sync (`no_u_sync`, `u_sync_once`, `disable_u_sync`). |

### 3. Validation & recording the sync

After porting upstream commits:

1. Run `cargo xtask lintrust` (`cargo clippy --workspace --all-targets -- -D warnings`).
2. Run `cargo xtask unittest` to verify the Rust library unit tests pass.
3. Run the affected functional and legacy tests:
   ```bash
   cargo xtask functionaltest --release <path/to/spec.lua>
   cargo xtask oldtest --release <test_name>
   ```
4. If `:help` documentation was modified, run `cargo xtask lintdoc`.
5. Update the **Last synced upstream commit** SHA in this section of `MAINTAIN.md` and commit.

### Event-boundary changes

Do not translate upstream raw pointers into long-lived Rust mutable references when callbacks can access the same object. The process, stream, channel, and loop interfaces deliberately use shared owners and interior mutability. Parser and reader state must be released before dispatch; queued work owns its payload and the storage it needs.

`CallbackHandle` registrations have distinct initialization, active, closing, and closed states. Never free native storage at the `uv_close` call: release its retained owner in the close callback. Failure and cancellation paths need the same treatment. The cross-thread inbox accepts only owned, thread-safe messages and serializes notification with shutdown.

On Windows, identify a TTY by its initialized native handle type, not a process stream's `fd` label, and stop reading before resetting its mode during close. A mode change can otherwise hand a new buffer to a console worker while Rust is dispatching an owned read batch. ConPTY output must still drain after logical close, so read resumption is gated by the native handle's active phase and requested read state.

Final cleanup releases request-owned queues before channels and the Lua/Vimscript runtimes. A shared Rust owner must not outlive a foreign runtime needed by its destructor.

For these changes, supplement the normal suites with:

```bash
cargo test -p event-lifetimes-tests
cargo +nightly-2026-11-16 miri test -p event-lifetimes-tests
cargo xtask functionaltest --release test/functional/core/job_spec.lua test/functional/core/channels_spec.lua
cargo xtask benchmark --release test/benchmark/event_spec.lua
```

Run affected functional cases and full CI-equivalent suites on Windows and Linux before publication. WSL tests need a native Linux checkout for POSIX permissions and Unix sockets. Compare the same tests and benchmarks against a preserved baseline binary using `--nvim-bin`; record pre-existing failures and skips separately. Miri exercises the production Rust ownership primitives with controlled callbacks, not the external C libraries.

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
    * `test.yml` also checks the FFI-free event lifetime package under a pinned Miri toolchain; the CI gate requires that job when Rust changes.
    * `release.yml`: Builds and publishes Linux `.tar.gz`/`.appimage`, macOS `.tar.gz`, and Windows `.zip`/`.msi` release assets via `cargo xtask dist`.
    * `zizmor.yml`: Static analysis of GitHub Actions workflows.
