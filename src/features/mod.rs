//! Features Module
//!
//! This module contains advanced editor features and functionality:
//! - Syntax highlighting and language support
//! - Text objects and advanced text manipulation
//! - Search and completion capabilities
//! - Language Server Protocol integration

pub mod completion;
pub mod lsp;
pub mod search;
pub mod syntax;
pub mod text_objects;

pub use completion::CommandCompletion;
pub use lsp::*;
pub use search::{SearchEngine, SearchResult};
pub use syntax::{AsyncSyntaxHighlighter, HighlightRange, Priority};
pub use text_objects::*;
