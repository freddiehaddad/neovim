//! Input Module
//!
//! This module handles all user input processing including:
//! - Event-driven architecture and event handling
//! - Keyboard input and key mapping
//! - User interaction events and commands

pub mod event_driven;
pub mod events;
pub mod keymap;

pub use event_driven::EventDrivenEditor;
pub use events::*;
pub use keymap::KeyHandler;
