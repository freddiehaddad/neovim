# Test Suites

See [`runtime/doc/dev_test.txt`](../runtime/doc/dev_test.txt) and [`BUILD.md`](../BUILD.md#running-tests) for documentation on running and writing tests:

- `cargo test --lib`: Rust unit tests across `src/`
- `cargo xtask functionaltest`: Lua functional test suite (`test/functional/`)
- `cargo xtask oldtest`: Legacy Vim test suite (`test/old/testdir/`)
- `cargo xtask benchmark --release`: Performance benchmark suite (`test/benchmark/`)

