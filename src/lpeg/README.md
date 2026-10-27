# LPeg - Parsing Expression Grammars For Lua

For more information,
see [Lpeg](https://www.inf.puc-rio.br/~roberto/lpeg/).

Ported to Rust from LPeg v1.1.0 (`lpcap.rs`, `lpcode.rs`, `lpcset.rs`, `lpprint.rs`, `lptree.rs`, `lpvm.rs`), registering `"lpeg"` via `luaL_register` to avoid conflicts with LuaJIT extensions.
