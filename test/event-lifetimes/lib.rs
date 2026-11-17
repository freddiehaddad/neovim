//! Exercise the production ownership primitives without linking libuv or Lua.
#![allow(dead_code)]

#[path = "../../src/nvim/event/inbox.rs"]
mod inbox;
#[path = "../../src/nvim/event/owned.rs"]
mod owned;
#[path = "../../src/nvim/event/read_buffer.rs"]
mod read_buffer;
#[path = "../../src/nvim/event/timer.rs"]
mod timer;
