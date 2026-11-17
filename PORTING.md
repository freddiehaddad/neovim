# Porting Neovim to Rust

How a 460,000-line C codebase with three decades of Vim history was converted into a pure-Rust project over 950+ commits without breaking existing configs, Lua plugins, or tests.

---

## Why From-Scratch Rewrites Fail

Almost every attempt to rewrite Vim or Neovim from scratch, whether by hand or with AI, ends the same way: a fast prototype that handles basic movement and editing, followed by a wall of thousands of subtle incompatibilities.

Neovim is not just a text buffer and a screen grid. Plugins and users depend on exact, often undocumented behavior across:
- Vimscript evaluation, scoping, and legacy Ex commands
- Two distinct regular expression engines (backtracking and NFA)
- Re-entrant autocommands that can close windows or delete buffers mid-operation
- Multi-grid UI protocols, terminal emulation (`libvterm`), and Msgpack-RPC semantics
- Direct LuaJIT C API and `libuv` event-loop integration

In an ecosystem like Neovim's, a 95% compatible rewrite is unusable if `lazy.nvim`, `telescope.nvim`, or built-in LSP breaks on edge cases. The only way to achieve 100% behavioral parity was to never lose a working editor at any point during the port.

---

## The Strategy: Plan First, Port Bottom-Up

Before modifying a single line of C code, we mapped out the repository's dependency graph and broke the port into explicit phases.

Instead of starting a separate Rust binary, we linked a Rust static library into the existing C build and replaced the codebase **one file at a time, from leaf modules to the root**:

1. **Leaf utilities first**: Character classification (`charset`), multibyte UTF-8 handling (`mbyte`), paths (`path`), strings (`strings`), memory allocation (`memory`), and data structures (`garray`, `hashtab`, `marktree`).
2. **Core editor subsystems**: Buffer management (`buffer`, `memline`), undo trees (`undo`), marks and extmarks (`mark`, `extmark`, `decoration`), syntax and regex (`syntax`, `regexp_bt`, `regexp_nfa`), and screen drawing (`grid`, `drawline`, `drawscreen`).
3. **Evaluation and scripting**: The Vimscript parser (`viml/`), evaluator (`eval/`), built-in `vim.fn` functions, LuaJIT bridge (`lua/`), and `nvim_*` API endpoints (`api/`).
4. **Event loop, UI, and entrypoint**: `libuv` process/stream/pty management (`event/`), Msgpack-RPC (`msgpack_rpc/`), the built-in terminal UI (`tui/`), and finally `main.rs`.

The rule for every single commit was strict:
1. Port one module (or one bounded piece of a large module) to Rust, exposing `extern "C"` symbols matching the C header.
2. Delete the `.c` file so the linker proves the Rust implementation is wired in.
3. Build and run the functional and legacy tests against the hybrid binary.
4. Commit only when green, then move to the next module.

---

## Tearing Down the C Scaffolding (950+ Commits)

Once the last `.c` file in `src/nvim/` was ported, the project was still a Rust codebase shaped like C and driven by CMake. Over several hundred subsequent commits, we dismantled the legacy scaffolding step by step:

- **Vendored C libraries ported to in-tree Rust**: `libvterm` (`src/nvim/vterm/`), `libtermkey` (`src/nvim/tui/termkey/`), `unibilium` (`src/unibilium/`), `utf8proc` (`src/nvim/mbyte_utf8proc.rs`), `libmpack` (`src/mpack/`), `lua-cjson` (`src/cjson/`), `LPeg` (`src/lpeg/`), `git xdiff` (`src/xdiff/`), `xxd` (`src/xxd/`), and `tee` (`src/tee/`).
- **Retiring `nlua0` and C code generators**: Replaced the C-based Lua bootstrap compiler and `.h.generated.h` C header generators with pure Rust and `cargo xtask`.
- **Retiring CMake and Zig**: Removed all `CMakeLists.txt`, `.cmake` scripts, and `build.zig` files in favor of a standard `Cargo.toml` workspace and a self-contained `build.rs` that fetches and builds the four remaining external C archives (`libuv`, `LuaJIT`, `luv`, `tree-sitter`) in `.deps/`.
- **Retiring LuaJIT FFI unit tests**: Upstream's `test/unit/` suite relied on LuaJIT FFI parsing C headers at runtime. We migrated those assertions into native Rust `#[cfg(test)]` unit tests (>1,000 tests) and deleted the C headers.
- **Data structure & ownership modernization**: Replaced C grow-arrays (`garray_T`), custom hash tables (`hashtab_T`), manual `xmalloc`/`xfree` lifecycles, C integer aliases (`c_int`, `c_long`), and over 1,000 `static mut` globals with Rust `Vec<T>`, `HashMap`/`BTreeMap`, `Box`/`CString` RAII, fixed-width primitives, and `GlobalState<T>`.

---

## What Went Wrong: Pitfalls & Hard Lessons

The port was not a smooth, linear march. Several classes of bugs, especially around FFI boundaries and Rust's aliasing rules, taught us hard lessons about what works and what backfires when porting a large C codebase.

### 1. The `--release` LLVM `noalias` Trap (The Clipboard Paste Hang)

After reaching zero `.c` files and passing the test suite, we ran a series of automated idiomatization passes (Phases 16-22) converting raw pointer parameters (`*mut T`) into mutable Rust references (`&mut T`). Debug builds compiled cleanly and passed functional tests.

In daily use on `--release` (`opt-level = 3`), however, yanking text in visual mode and pressing `p` with `set clipboard=unnamedplus` hung the editor until `Ctrl+C` was pressed.

**Root cause**: `proc_wait(proc_: *mut Proc, ...)` had been changed to take `proc_: &mut Proc`. In Rust, `&mut T` carries LLVM's `noalias` attribute, promising the optimizer that no other pointer can modify `*proc_` while the function runs. Inside `proc_wait`, Neovim polls `libuv` (`uv_run`) in a loop until `proc_.refcount == 1`. Because `proc_.refcount` is decremented asynchronously by `libuv` C callbacks via raw pointers, LLVM's optimizer deduced under `noalias` that `proc_.refcount == 1` could *never* become true and constant-folded the loop into `while !got_int`, spinning forever until `Ctrl+C` set `got_int`.

### 2. Overlapping `&mut` Borrows in Re-Entrant Callbacks

In C, holding a pointer to a parent struct (`RemoteUI` or `Stream`) while passing a pointer to one of its fields (`PackerBuffer` or `RStream`) into a callback that accesses the parent via `container_of` / `anydata` is routine.

When both pointers were converted to `&mut` references (`ui_flush_callback` in `api/ui.rs` and `read_event` in `event/rstream.rs`), creating a second `&mut` to the parent while the child `&mut` was still live violated Rust's aliasing rules. Under `--release` with ThinLTO, LLVM reordered buffer pointer writes across the flush callback and silently corrupted the Msgpack-RPC byte stream in `multigrid_spec.lua`.

### 3. Pointer Invalidation Across Autocommands

In Neovim, seemingly simple functions like `win_close`, `buf_write`, or `win_enter_ext` fire user autocommands (`WinClosed`, `BufWritePre`, `WinEnter`). An autocommand callback can execute arbitrary Lua or Vimscript, including closing other windows or freeing the very buffer being operated on.

Converting `*mut WinInfoInfo` or `*mut FileBuf` into a top-of-function `let wp = unsafe { &mut *wp };` binding across an autocommand boundary meant holding a live Rust reference to freed memory, even when the code later checked `if !win_valid(wp)`. We had to establish a strict rule: never hold `&mut` references to windows, buffers, or tabpages across autocommand, event-loop, or deallocator boundaries.

### 4. Syntax-Driven `unsafe` Narrowing on `Copy` Structs

An automated cleanup pass narrowed `let p = unsafe { &mut *ptr };` into `let p = &mut unsafe { *ptr };` to shrink the lexical span of `unsafe`.

On non-`Copy` types, the compiler rejected this immediately. On `Copy` structs (specifically `regsave_T` in `regexp_bt::regstack_push`), `unsafe { *ptr }` silently copied the struct by value onto the stack and took a mutable reference to the temporary copy. Mutations to `p` vanished at the end of the function, corrupting the backtracking regex stack (`E44: Corrupted regexp program`) whenever syntax highlighting ran.

### 5. Windows LLP64 `libuv` Struct Layouts

Early in the port, hand-written `#[repr(C)]` definitions for `libuv` handles (`uv_fs_t`, `uv_process_options_t`, `uv_stat_t`) assumed Unix field ordering and 64-bit `c_long`. On 64-bit Windows (LLP64, where `c_long` is 32 bits and `uv_fs_t` embeds Win32-specific fields before `.result` and `.ptr`), every filesystem call read garbage offsets and crashed on startup until we audited and locked down platform-exact FFI layouts.

---

## Reentrant Process and Channel Ownership

The process-waiting refactor addresses the ownership contract, not just the spelling of pointer parameters:

- `Proc` and `Stream` own shared Rust state. Callback-mutated scalars use interior mutability, and polling does not retain exclusive references to processes, streams, channels, or the loop.
- `CallbackHandle` retains an address-stable allocation from native registration until the native close callback. Queued reads and writes also retain their owners. Starting close and completing close are separate operations, including on initialization failure.
- Channel lookup returns `Rc<Channel>`. These physical owners do not replace Neovim's logical reference counts or determine when channel cleanup and process-exit callbacks occur.
- RPC decoding produces owned messages before dispatch. Stream dispatch owns each read batch, so nested polling cannot overwrite the active callback's borrowed bytes. Ordinary read dispatch reuses its allocation.
- The main-thread loop is not a globally accessible mutable reference. Worker notifications contain owned data or IDs, not editor pointers or `Rc` values; the inbox synchronizes wakeups with shutdown.

The FFI implementation still requires explicit unsafe contracts. In particular, libuv's opaque `data` points to the registered state, not to the Rust handle wrapper. Callbacks recover that state through the typed ownership helper. Other FFI wrappers must still be reviewed according to their own lifetime and reentrancy requirements; this refactor is not a claim that every external boundary is proven sound.

Native read-buffer lifetimes must also be respected. Windows TTY mode changes can restart reading and allocate a buffer for a console worker, so closing a readable TTY stops reading before resetting its mode. Identify the TTY by its initialized native handle type: process pipes can carry stdout/stderr labels that refer to a different handle in the parent. An owned callback batch does not permit replacing an allocation still leased to native I/O.

Logical stream closure precedes native handle closure. In particular, ConPTY shutdown must keep draining output until the console worker finishes. Read resumption checks the native handle's active phase and the requested read state, not just the logical closed flag.

The `event-lifetimes-tests` package imports the production ownership, read-buffer, and inbox modules without linking libuv or Lua. Its tests can run under Miri. Native functional tests remain necessary for real callbacks, platform-specific process teardown, error paths, and compatibility. Runner-side RPC deadlines bound hang-sensitive tests; a timer inside a stuck editor cannot serve as its watchdog.

## Where Things Stand & What's Next

The tracked editor implementation is Rust (`edition = "2024"`), with external C libraries built separately as dependencies. Editor graph objects already use shared `Rc` handles and interior mutability; they should not be described as unported raw-pointer lists.

Compatibility remains the primary invariant, not a conclusion inferred from test counts. CI runs Rust unit tests and functional/legacy suites across multiple platforms and optimized configurations. See [BUILD.md](BUILD.md) for current commands, [AGENTS.md](AGENTS.md) for implementation rules, and [MAINTAIN.md](MAINTAIN.md) for the recorded upstream synchronization point.

Further modernization should enforce one concrete invariant at a time: validated representations, bounded borrows around callbacks, and explicit ownership at remaining FFI boundaries. Generational IDs or parser redesigns need a demonstrated benefit beyond the guarantees already provided by shared handles and slice-based APIs.

Published performance measurements and their upstream revision are in [README.md](README.md#performance). Ownership refactors do not imply a speedup: compare relevant release workloads against the unchanged port, including process waits, persistent RPC traffic, and stream draining.

