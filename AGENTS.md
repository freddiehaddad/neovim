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

## Critical Rust Soundness & Architecture Invariants

The Neovim core uses `#![forbid(unsafe_code)]` / `#![deny(unsafe_code)]` across non-FFI modules, modeling editor graph objects (`WinPtr`, `BufPtr`, `TabpagePtr`, `FramePtr`) with `Rc<T>` and interior mutability (`Cell`, `CloneCell`, `RefCell`). Only genuine C-ABI boundaries (`src/nvim/lua/`, `src/nvim/event/`, `src/nvim/os/`, and exported LuaJIT `ffi.C` shims) use `unsafe` FFI and `#[repr(C)]`.

Follow these rules strictly when modifying `src/nvim/`:

1. **Never hold `RefCell` (`Ref` / `RefMut`) guards across reentrant boundaries**:
   - Functions that trigger autocommands (`apply_autocmds*`), Lua callbacks (`nlua_call*`), Vimscript evaluation (`eval*`, `do_cmdline*`), UI prompts (`msg_putchar`, `do_more_prompt`, `ui_refresh`), regex execution (`vim_regexec*`), or `libuv` event loop ticks (`uv_run`, `Loop::poll_events`) can re-enter any editor subsystem.
   - Scope `RefCell::borrow()` and `RefCell::borrow_mut()` guards to pure synchronous data access; clone `Rc` handles, copy small values, or step by index before invoking callbacks or UI output, and never take `borrow_mut()` for read-only queries.
2. **Use shared handles (`WinPtr`, `BufPtr`, `TabpagePtr`, `FramePtr`) and `Deref`**:
   - Access current editor state via `curwin()`, `curbuf()`, `curtab()`, `firstwin()`, `lastwin()`, `topframe()`, and `first_tabpage()`.
   - `WinPtr`, `BufPtr`, `TabpagePtr`, and `FramePtr` implement `Deref` for non-null access and `.opt()` for `Option<&T>`. Reserve `.borrow()` and `.borrow_mut()` for `RefCell` fields so interior mutability borrows remain explicit and auditable.
3. **Use local stack buffers or owned values instead of global scratch buffers**:
   - Format transient messages and paths into local stack arrays (`[0u8; IOSIZE]`, `[0u8; MAXPATHL]`) or owned `Vec<u8>` / `CString` values rather than shared mutable state in `EditorState`.
4. **Restrict `#[repr(C)]` and raw pointers to true C-ABI / FFI boundaries**:
   - Keep `#[repr(C)]` only on types that cross C FFI (`lua/ffi.rs`, `event/ffi.rs`, `os/`, swapfile/undo binary headers, and `#[unsafe(no_mangle)]` / `#[unsafe(export_name = ...)]` LuaJIT `ffi.C` shims).
   - Note that LuaJIT's `ffi.C` tests declare several types (such as `Error`) as 0-byte `typedef struct {}`; exported `extern "C"` shims must not write larger Rust structs through such pointers.
   - In `src/nvim/event/`, `libuv` callbacks mutate `Proc` and `Stream` fields through raw C pointers (`handle.data`). Polling loops must take `*mut Proc` and read shared state via `VolatileCell` / `std::ptr::read_volatile`, never holding `&mut Proc` across `uv_run`.
5. **Preserve NUL-termination invariants on C-string buffers**:
   - Use `CStrSlice`, `OwnedSlice`, `RcSlice`, `CString`, or `c"..."` literals for NUL-terminated strings; never pass `Vec::new().as_ptr()` to C functions expecting a valid NUL-terminated `*const c_char`.
6. **Always validate nontrivial core changes in `--release` mode**:
   - Test core changes with `cargo xtask functionaltest --release` to validate both optimized codegen and runtime `RefCell` invariants.

## Syncing Upstream `neovim/neovim` Commits

- Upstream C files `src/nvim/<path>.c` and `src/nvim/<path>.h` map 1:1 to `src/nvim/<path>.rs`.
- Upstream `runtime/` and `test/` changes apply directly.
- See [MAINTAIN.md](./MAINTAIN.md) for the upstream sync checklist and the last synced commit SHA.

## AI Disclosure

If AI was used for a commit, add a generic `AI-assisted` trailer to the commit message.
Do NOT add any info mentioning the particular model or the AI service.
Do NOT list an AI service as the author name/email.

