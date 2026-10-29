# AGENTS.md

Instructions for AI coding agents working in this repository.

## Project Overview

This repository is a pure-Rust port of [Neovim](https://github.com/neovim/neovim) (`edition = "2024"`).
- **Zero `.c` or `.h` files exist in the repository.** Do not add C source/header files, CMake files, or Zig build files.
- **100% behavioral, API, and plugin compatibility with upstream Neovim** is the primary invariant. Every change must preserve LuaJIT C API compatibility, Msgpack-RPC wire compatibility, and Vimscript/editor semantics.
- The workspace builds three binaries (`nvim`, `nvim-tee`, `xxd`), the `neovim` library crate (`src/lib.rs`), and the `xtask` build/test runner (`src/xtask/main.rs`).

## Build, Test, and Lint Commands

Always use `cargo` and `cargo xtask` (never `cmake`, `make`, or `ninja`):

```bash
# Compile and run Rust unit tests
cargo test --lib

# Lint Rust (clippy -D warnings), Lua (stylua), error codes, and :help docs
cargo xtask lint
cargo xtask lintrust

# Run functional tests (pass --release to test the optimized binary)
cargo xtask functionaltest test/functional/<path>_spec.lua
cargo xtask functionaltest --release test/functional/<path>_spec.lua

# Run legacy Vim tests
cargo xtask oldtest test_<name>
cargo xtask oldtest --release test_<name>

# Run performance benchmarks
cargo xtask benchmark --release
```

## Critical Rust Soundness & FFI Invariants

The Neovim core interacts heavily with global editor state (`curwin`, `curbuf`, `firstwin`, `firstbuf`, `g_st()`), Lua/Vimscript callbacks, `libuv` C callbacks, and arena/pool allocators. Violating Rust's aliasing rules (`&mut T` carries LLVM `noalias` in `--release` builds) causes miscompilations that only manifest under `opt-level = 3`.

Follow these rules strictly when modifying `src/nvim/`:

1. **Never hold `&mut T` across callbacks, `uv_run`, autocmds, or deallocators**:
   - Functions that trigger autocommands (`apply_autocmds*`), Lua callbacks (`nlua_call*`), Vimscript evaluation (`eval*`, `do_cmdline*`), window/buffer switches, or `libuv` event loop ticks (`uv_run`, `Loop::poll_events`) can mutate or free windows (`WinInfoInfo`), buffers (`FileBuf`), tabpages (`Tabpage`), and processes (`Proc`).
   - Keep raw pointers (`*mut WinInfoInfo`, `*mut FileBuf`, `*mut Proc`, `*mut Terminal`) across those boundaries and re-borrow locally only for the duration of a synchronous, non-reentrant access.
2. **Never hold `&mut T` while polling fields mutated asynchronously by `libuv`**:
   - In `src/nvim/event/`, `libuv` callbacks mutate `Proc`, `Stream`, and `MultiQueue` fields through raw C pointers (`handle.data`). Polling loops (such as `proc_wait` or `loop_uv_run`) must take `*mut Proc` and read shared state via `std::ptr::read_volatile`—never `&mut Proc`, which allows LLVM to constant-fold loop conditions under `noalias`.
3. **Never create overlapping `&mut` references to parent and child structs**:
   - Do not hold `&mut Stream` (`RStream`) while passing `&mut RStream` (`stream.data`) into a callback (`read_event`), or hold `&mut PackerBuffer` while mutating the owning `RemoteUI` via `packer.anydata` (`ui_flush_callback`). Re-derive child pointers from the raw parent pointer as needed.
4. **Never narrow `unsafe { &mut *ptr }` into `&mut unsafe { *ptr }` on `Copy` structs**:
   - `&mut unsafe { *ptr }` copies `*ptr` by value into a stack temporary and takes a mutable reference to the temporary, silently discarding mutations. Always write `unsafe { &mut *ptr }`.
5. **Preserve NUL-termination invariants on C-string buffers**:
   - Functions expecting NUL-terminated `*const c_char` / `*mut c_char` (such as `msg_multiline`, `ml_get`, and regex/parser cursors) must never be passed `Vec::as_ptr()` on an empty or non-NUL-terminated `Vec<c_char>` (`Vec::new().as_ptr()` is a dangling `0x1` pointer).
6. **Always validate nontrivial core changes in `--release` mode**:
   - Debug builds (`opt-level = 0`) do not exploit LLVM `noalias` optimizations. When modifying pointer/reference signatures or event/buffer/window lifetimes, test with `cargo xtask functionaltest --release`.

## Syncing Upstream `neovim/neovim` Commits

- Upstream C files `src/nvim/<path>.c` and `src/nvim/<path>.h` map 1:1 to `src/nvim/<path>.rs`.
- Upstream `runtime/` and `test/` changes apply directly.
- See [MAINTAIN.md](./MAINTAIN.md) for the upstream sync checklist and the last synced commit SHA.

## AI Disclosure

If AI was used for a commit, add a generic `AI-assisted` trailer to the commit message.
Do NOT add any info mentioning the particular model or the AI service.
Do NOT list an AI service as the author name/email.

