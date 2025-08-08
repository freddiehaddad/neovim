// Core functionality
pub mod core;

// User interface
pub mod ui;

// Input handling
pub mod input;

// Configuration management
pub mod config;

// Features
pub mod features;

// Utilities
pub mod utils;

// Re-exports
pub use core::{buffer::Buffer, editor::Editor};
pub use input::{event_driven::EventDrivenEditor, events::*, keymap::KeyHandler};
pub use ui::terminal::Terminal;
