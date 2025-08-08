//! Configuration System
//!
//! This module handles:
//! - Editor configuration loading and validation
//! - Configuration file watching and hot reloading
//! - Theme and keymap configuration management

pub mod editor;
pub mod theme;
pub mod watcher;

pub use editor::EditorConfig;
pub use theme::ThemeConfig;
pub use watcher::ConfigWatcher;
