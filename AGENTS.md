# AGENTS.md

Instructions for AI coding agents working in this repository.

## Project Overview

This repository is a pure-Rust port of [Neovim](https://github.com/neovim/neovim) (`edition = "2024"`).
- **Zero `.c` or `.h` files exist in the repository.** Do not add C source/header files, CMake files, or Zig build files.
- **100% behavioral, API, and plugin compatibility with upstream Neovim** is the primary invariant. Every change must preserve LuaJIT C API compatibility, Msgpack-RPC wire compatibility, and Vimscript/editor semantics.
- The workspace builds three application binaries (`nvim`, `tee`, `xxd`), the `neovim` library crate (`src/nvim/lib.rs`), the `xtask` runner, and the FFI-free `event-lifetimes-tests` package.

## Build, Test, and Lint Commands

Always use `cargo` and `cargo xtask` (never `cmake`, `make`, or `ninja`):

```bash
# Compile and run Rust unit tests
cargo test --lib
cargo test -p event-lifetimes-tests

# Check production ownership primitives without linking libuv or Lua
cargo +nightly-2026-11-16 miri test -p event-lifetimes-tests

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
cargo xtask benchmark --release test/benchmark/event_spec.lua
```

## Critical Rust Soundness & Architecture Invariants

The Neovim core uses `#![forbid(unsafe_code)]` / `#![deny(unsafe_code)]` across non-FFI modules, modeling editor graph objects (`WinPtr`, `BufPtr`, `TabpagePtr`, `FramePtr`) with `Rc<T>` and interior mutability (`Cell`, `CloneCell`, `RefCell`). Only genuine C-ABI boundaries (`src/nvim/lua/`, `src/nvim/event/`, `src/nvim/os/`, and exported LuaJIT `ffi.C` shims) use `unsafe` FFI and `#[repr(C)]`.

Follow these rules strictly when modifying `src/nvim/`:

1. **Never hold `RefCell` (`Ref` / `RefMut`) guards across reentrant boundaries**:
   - Functions that trigger autocommands (`apply_autocmds*`), Lua callbacks (`nlua_call*`), Vimscript evaluation (`eval*`, `do_cmdline*`), UI prompts (`msg_putchar`, `do_more_prompt`, `ui_refresh`), regex execution (`vim_regexec*`), or `libuv` event loop ticks (`uv_run`, `Loop::poll_events`) can re-enter any editor subsystem.
   - Scope `RefCell::borrow()` and `RefCell::borrow_mut()` guards to pure synchronous data access; clone `Rc` handles, copy small values, or step by index before invoking callbacks or UI output, and never take `borrow_mut()` for read-only queries.
2. **Use shared handles (`WinPtr`, `BufPtr`, `TabpagePtr`, `FramePtr`), `Deref`, and domain subsystem accessors**:
   - Access current editor state via `curwin()`, `curbuf()`, `curtab()`, `firstwin()`, `lastwin()`, `topframe()`, and `first_tabpage()`, and mutate `curwin` exclusively via `set_curwin()` so `CURWIN_FFI` stays synchronized.
   - Single-module globals belong in their owning module's `thread_local!` state struct; cross-module globals live in the 9 decentralized domain subsystem structs (`workspace_state()` in `globals.rs`, `msg_state()` in `message.rs`, `exc_state()` in `ex_eval.rs`, `screen_state()` in `drawscreen.rs`, `input_state()` in `input.rs`, `cmdline_state()` in `ex_getln.rs`, `autocmd_state()` in `autocmd.rs`, `search_state()` in `search.rs`, `edit_state()` in `normal.rs`).
   - Do not expose `pub RefCell<...>` fields on `FileBuf`, `WinInfoInfo`, `TabPageInfo`, or domain subsystem structs; keep `RefCell` fields private behind reentrancy-safe domain methods, use `Cell<T>` / `CloneCell<T>` for scalars and shared slices (`VisualinfoCell`, `FmarkCell`, `LcsCharsCell`, `TabstopArray`), and use `#[must_use]` RAII guards (`ScopedCellDeltaGuard`, `ScopedCellValueGuard`) for lexical state counter/value scopes.
   - `WinPtr`, `BufPtr`, `TabpagePtr`, and `FramePtr` implement `Deref` for non-null access and `.opt()` for `Option<&T>`. Reserve `.borrow()` and `.borrow_mut()` for `RefCell` fields so interior mutability borrows remain explicit and auditable.
3. **Use local stack buffers or owned values instead of global scratch buffers**:
   - Format transient messages and paths into local stack arrays (`[0u8; IOSIZE]`, `[0u8; MAXPATHL]`) or owned `Vec<u8>` / `CString` values rather than shared mutable state.
4. **Restrict `#[repr(C)]` and raw pointers to true C-ABI / FFI boundaries**:
   - Keep `#[repr(C)]` only on types that cross C FFI (`lua/ffi.rs`, `event/ffi.rs`, `os/`, swapfile/undo binary headers, and `#[unsafe(no_mangle)]` / `#[unsafe(export_name = ...)]` LuaJIT `ffi.C` shims).
   - Note that LuaJIT's `ffi.C` tests declare several types (such as `Error`) as 0-byte `typedef struct {}`; exported `extern "C"` shims must not write larger Rust structs through such pointers.
   - In `src/nvim/event/`, `Proc` and `Stream` are shared handles to address-stable, interior-mutable state. Waiting/draining APIs take shared handles, never `&mut` references to callback-visible state or its containing `Channel`/`Loop`. Casting an incoming `&mut T` to a raw pointer does not remove its exclusivity contract; volatile accesses do not repair aliasing or provide thread synchronization.
   - Libuv storage is accessed through `CallbackHandle::as_ptr()` or `Loop::uv_ptr()`, never by casting the Rust wrapper's address or assuming its first field matches C storage. `register` retains its containing `Rc` allocation after native initialization; callbacks recover the correct state type with `from_callback`. Initiate close once and release the registration only in the native close callback with `finish_close`. Pending Rust events/requests must retain their own owners. A failed initialization, cancelled event, or shutdown path must obey the same lifetime rules.
   - `main_loop()` returns shared main-thread state, not `&'static mut Loop`. Channel lookup returns `Rc<Channel>`. Keep registry/reader/parser borrows local; snapshot decoded RPC messages and callback payloads before dispatch. Logical Neovim reference counts and close-event timing are separate from physical Rust ownership: never substitute `Rc::strong_count()` for protocol counters.
   - During final cleanup, cancel queued requests before dropping channel registry owners or destroying the Lua/Vimscript runtimes. Cancel shared oneshot payloads across all queue aliases, not just individual entries. Keep channel owners alive and registry borrows released during cancellation; a channel destructor cannot break a cycle through requests in its own queue.
   - Each loop context is thread-local. Workers receive a `ThreadSender` captured from the owning loop and send only owned `ThreadEvent` payloads through its synchronized inbox. Never send `Rc`, `NvimString`, arbitrary `Event` values, or pointers to editor state across threads. The inbox lock covers both wakeup and shutdown; do not restore a blanket `unsafe impl Sync` for the editor loop.
   - Read dispatch owns a `ReadBatch`; callbacks borrow its bytes rather than mutable stream storage. Preserve unread bytes across reentry and reuse the allocation on ordinary dispatch. Do not retain a `RefCell` guard, whole-object mutable reference, or pointer into a replaceable buffer while calling user code.
   - On Windows, identify a TTY by its initialized native handle type, not a process stream's `fd` label. Stop TTY reading before resetting its mode during close: libuv can restart reading and lease a buffer to a console worker during a mode change, and that allocation must not be replaced while the read is outstanding.
   - Distinguish logical stream closure from native handle closure. ConPTY shutdown still drains output after logical close; permit read resumption only while the native handle is active and reading is wanted, never once native close begins.
5. **Preserve NUL-termination invariants on C-string buffers and use `Memline` chunk APIs**:
   - Use `RcCStr`, `RcSlice`, `CString`, or `c"..."` literals for NUL-terminated strings; never pass `Vec::new().as_ptr()` to C functions expecting a valid NUL-terminated `*const c_char`.
   - `Memline` (`src/nvim/memline.rs`) stores in-memory buffer lines in `Vec<LineChunk>` (400–800 `LineEntry` values per chunk, each holding `RcCStr` text and a mark bit, with `start_lnum`, `totalsize`, and cached `swap_blocks`). `RcCStr` uses `Arc<CStr>`, allowing chunks to be `Send + Sync`. Dirty chunks serialize into 4KB `PointerBlock`/`DataBlock` pages when syncing `.swp` files (`ml_sync_swap_blocks`). Use `with_ml_buf_line` and `with_ml_buf_bytes_with_nul` for borrowed line slice access.
6. **Keep pure compute engines stateless (`Send + Sync`) for `std::thread::scope` parallelism**:
   - Compiled regex programs (`NfaRegprog`, `BtRegprog` in `src/nvim/regexp.rs`) are immutable (`Send + Sync`), storing per-run NFA state (`lastlist_buf`, `rec_listbuf`) in thread-local scratch buffers so pure single-line patterns (`ml_try_mark_global_lines`, `ml_try_vimgrep_lines`), fuzzy candidate scoring (`src/nvim/fuzzy.rs`), diff `linematch` (`src/nvim/linematch.rs`), and spell soundalike scoring (`src/nvim/spellsuggest.rs`) can execute across worker threads via `std::thread::scope` while keeping autocommands, Lua callbacks, and UI updates on the main thread.
7. **Always validate nontrivial core changes in `--release` mode**:
   - Test core changes with `cargo xtask functionaltest --release` to validate both optimized codegen and runtime `RefCell` invariants.
   - For event/ownership changes, also run the Rust lifetime package under Miri and the affected process, channel, RPC, thread, timer, clipboard, and terminal cases on Windows and Linux. Use a native Linux checkout for WSL tests, not a Windows-mounted filesystem with different permission/socket behavior.
   - Add deterministic tests for the invariant being changed: retention through close callbacks, initialization failure, cancellation, reentry, and shutdown races as applicable. Use handshakes/barriers and test-runner deadlines, not timing-dependent sleeps or a timeout inside an editor that may be hung. Do not turn regressions into skips or weaker assertions.
   - Rust unit tests that need editor initialization must retain the `main_init::test_editor()` guard for their scope. It initializes and closes native event resources on that test's own thread; do not share one initialized loop across Rust test threads or drop its storage before native teardown.
   - Miri covers the Rust-only ownership primitives, not libuv/Lua or the entire editor. Review each remaining unsafe boundary's allocation ownership, address stability, aliasing, callback lifetime, and threading obligations. Passing CI or reducing the number of `unsafe` blocks is not a soundness proof.
   - Measure hot-path changes with relevant release benchmarks against the unchanged binary. Preserve externally observable ordering and allow legitimate interleavings between independent streams; do not claim a performance improvement without measurements.

## Syncing Upstream `neovim/neovim` Commits

- Upstream C files `src/nvim/<path>.c` and `src/nvim/<path>.h` map 1:1 to `src/nvim/<path>.rs`.
- Upstream `runtime/` and `test/` changes apply directly.
- See [MAINTAIN.md](./MAINTAIN.md) for the upstream sync checklist and the last synced commit SHA.

## AI Disclosure

If AI was used for a commit, add a generic `AI-assisted` trailer to the commit message.
Do NOT add any info mentioning the particular model or the AI service.
Do NOT list an AI service as the author name/email.

