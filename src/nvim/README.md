# Neovim Core (`src/nvim/`)

This directory contains the pure-Rust implementation of the Neovim editor core. Each `.rs` module corresponds 1:1 to its original upstream `src/nvim/<module>.c` counterpart.

For architecture and developer documentation, see:

- [`AGENTS.md`](../../AGENTS.md) — Rust soundness invariants, FFI boundaries, and build/test commands
- [`runtime/doc/dev_arch.txt`](../../runtime/doc/dev_arch.txt) — Neovim internal architecture
- [`runtime/doc/dev_tools.txt`](../../runtime/doc/dev_tools.txt) — Developer tools and debugging
- [`runtime/doc/dev.txt`](../../runtime/doc/dev.txt) — General developer guidelines

