//! Core Module
//!
//! This module contains the fundamental components of the editor:
//! - Buffer management and text manipulation
//! - Editor state and core logic  
//! - Window management and display layout
//! - Cursor movement and text editing modes

pub mod buffer;
pub mod editor;
pub mod mode;
pub mod window;

pub use buffer::Buffer;
pub use editor::Editor;
pub use mode::{Mode, Position, Selection};
pub use window::{SplitDirection, Window, WindowManager};
