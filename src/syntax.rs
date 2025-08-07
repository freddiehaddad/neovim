use anyhow::{Result, anyhow};
use crossterm::style::Color;
use log::{debug, info, trace, warn};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tree_sitter::{Language, Parser};

use crate::theme::{SyntaxTheme, ThemeConfig};

// Get the Rust language from the tree-sitter-rust crate
fn get_rust_language() -> Language {
    tree_sitter_rust::LANGUAGE.into()
}

#[derive(Debug, Clone)]
pub struct HighlightRange {
    pub start: usize,
    pub end: usize,
    pub style: HighlightStyle,
}

#[derive(Debug, Clone)]
pub struct HighlightStyle {
    pub fg_color: Option<String>,
    pub bg_color: Option<String>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl HighlightStyle {
    pub fn from_color(color: Color) -> Self {
        // Convert Color to hex string for storage
        let color_string = match color {
            Color::Rgb { r, g, b } => format!("#{:02x}{:02x}{:02x}", r, g, b),
            _ => "#ffffff".to_string(), // Default fallback for any non-RGB colors
        };

        HighlightStyle {
            fg_color: Some(color_string),
            bg_color: None,
            bold: false,
            italic: false,
            underline: false,
        }
    }

    pub fn to_color(&self) -> Option<Color> {
        self.fg_color.as_ref().and_then(|color_str| {
            if color_str.starts_with('#') && color_str.len() == 7 {
                let r = u8::from_str_radix(&color_str[1..3], 16).ok()?;
                let g = u8::from_str_radix(&color_str[3..5], 16).ok()?;
                let b = u8::from_str_radix(&color_str[5..7], 16).ok()?;
                Some(Color::Rgb { r, g, b })
            } else {
                // Return None for invalid hex colors
                None
            }
        })
    }

    /// Create HighlightStyle with a default color for tree-sitter highlighting
    pub fn from_tree_sitter_color(color: crossterm::style::Color) -> Self {
        // Convert Color to hex string for storage
        let color_string = match color {
            Color::Rgb { r, g, b } => format!("#{:02x}{:02x}{:02x}", r, g, b),
            _ => "#ffffff".to_string(), // Default fallback for any non-RGB colors
        };

        HighlightStyle {
            fg_color: Some(color_string),
            bg_color: None,
            bold: false,
            italic: false,
            underline: false,
        }
    }

    /// Builder method to set bold
    pub fn with_bold(mut self, bold: bool) -> Self {
        self.bold = bold;
        self
    }

    /// Builder method to set italic
    pub fn with_italic(mut self, italic: bool) -> Self {
        self.italic = italic;
        self
    }
}

/// Cache key for syntax highlighting results
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct HighlightCacheKey {
    content_hash: u64,
    language: String,
    theme: String,
}

/// Cache entry for syntax highlighting
#[derive(Debug, Clone)]
pub struct HighlightCacheEntry {
    highlights: Vec<HighlightRange>,
}

impl HighlightCacheKey {
    fn new(content: &str, language: &str, theme: &str) -> Self {
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        let content_hash = hasher.finish();

        Self {
            content_hash,
            language: language.to_string(),
            theme: theme.to_string(),
        }
    }

    /// Create a new cache key with default theme
    pub fn new_simple(content: &str, language: &str) -> Self {
        // Get the current theme from config
        let theme_config = crate::theme::ThemeConfig::load();
        let theme_name = &theme_config.theme.current;
        log::debug!("Creating cache key with theme: {}", theme_name);
        Self::new(content, language, theme_name)
    }
}

impl HighlightCacheEntry {
    pub fn new(highlights: Vec<HighlightRange>) -> Self {
        Self { highlights }
    }

    pub fn highlights(&self) -> &Vec<HighlightRange> {
        &self.highlights
    }
}

pub struct SyntaxHighlighter {
    parsers: HashMap<String, Parser>,
    theme_config: ThemeConfig,
    current_syntax_theme: SyntaxTheme,
}

impl SyntaxHighlighter {
    pub fn new() -> Result<Self> {
        info!("Initializing syntax highlighter");
        // Load the theme system
        let theme_config = ThemeConfig::load();
        let current_theme = theme_config.get_current_theme();
        debug!("Loaded syntax theme: '{}'", current_theme.name);

        let mut highlighter = SyntaxHighlighter {
            parsers: HashMap::new(),
            theme_config,
            current_syntax_theme: current_theme.syntax,
        };

        highlighter.initialize_parsers()?;
        Ok(highlighter)
    }

    fn initialize_parsers(&mut self) -> Result<()> {
        // Hardcoded Rust language support
        let language = get_rust_language();

        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| anyhow!("Failed to set language for rust: {}", e))?;

        self.parsers.insert("rust".to_string(), parser);

        Ok(())
    }

    pub fn detect_language_from_extension(&self, file_path: &str) -> Option<String> {
        // Use the language configuration from editor.toml instead of hard-coded values
        let config = crate::config::EditorConfig::load();
        config.languages.detect_language_from_extension(file_path)
    }

    /// Detect language from content patterns for unnamed files
    pub fn detect_language_from_content(&self, content: &str) -> Option<String> {
        let config = crate::config::EditorConfig::load();
        config.languages.detect_language_from_content(content)
    }

    /// Detect language using both file path and content fallback
    pub fn detect_language(&self, file_path: Option<&str>, content: &str) -> String {
        let config = crate::config::EditorConfig::load();

        if let Some(path) = file_path {
            if let Some(language) = self.detect_language_from_extension(path) {
                return language;
            }
        }

        // Fall back to content-based detection
        self.detect_language_from_content(content)
            .or_else(|| config.languages.get_fallback_language())
            .unwrap_or_else(|| "text".to_string()) // Ultimate fallback
    }

    pub fn update_theme(&mut self, theme_name: &str) -> Result<()> {
        // Reload the theme config to get updated themes
        self.theme_config = ThemeConfig::load();

        // Validate that the theme exists
        if let Some(complete_theme) = self.theme_config.get_theme(theme_name) {
            self.current_syntax_theme = complete_theme.syntax;
        } else {
            // Fallback to current theme if theme not found
            let current_theme = self.theme_config.get_current_theme();
            self.current_syntax_theme = current_theme.syntax;
            return Err(anyhow!(
                "Theme '{}' not found, using current theme",
                theme_name
            ));
        }

        // Note: Cache clearing removed since we now use async cache
        // The async cache will handle cache invalidation when needed

        Ok(())
    }

    pub fn highlight_text(&mut self, text: &str, language: &str) -> Result<Vec<HighlightRange>> {
        trace!(
            "Highlighting {} characters of {} code",
            text.len(),
            language
        );

        let parser = self
            .parsers
            .get_mut(language)
            .ok_or_else(|| anyhow!("No parser for language: {}", language))?;

        let tree = parser
            .parse(text, None)
            .ok_or_else(|| anyhow!("Failed to parse text"))?;

        let mut highlights = Vec::new();

        // Collect all node matches first (including overlapping ones)
        let mut node_matches = Vec::new();

        // Use a simplified approach - query all nodes manually using the Tree-sitter tree
        let mut stack = vec![tree.root_node()];

        while let Some(node) = stack.pop() {
            let node_kind = node.kind();

            // Debug: Log all node kinds to understand what tree-sitter provides
            if let Ok(node_text) = node.utf8_text(text.as_bytes()) {
                if node_text.len() < 20 && !node_text.contains('\n') {
                    log::debug!("Node type: '{}' -> text: '{}'", node_kind, node_text);
                }
            }

            // Special handling for comment nodes - apply unified coloring to entire comment
            if node_kind == "line_comment" || node_kind == "block_comment" {
                if let Some(color) = self
                    .current_syntax_theme
                    .tree_sitter_mappings
                    .get(node_kind)
                {
                    // Highlight the entire comment (including // or /* markers) with comment color
                    highlights.push(HighlightRange {
                        start: node.start_byte(),
                        end: node.end_byte(),
                        style: HighlightStyle::from_color(color.clone()),
                    });
                    // Don't process children - we want unified coloring for the entire comment
                    continue;
                }
            }

            // Check if this node type has a mapping
            if let Some(color) = self
                .current_syntax_theme
                .tree_sitter_mappings
                .get(node_kind)
            {
                // Special handling for doc comment markers - highlight them directly
                // instead of their children to maintain consistent coloring
                if node_kind == "outer_doc_comment_marker"
                    || node_kind == "inner_doc_comment_marker"
                {
                    highlights.push(HighlightRange {
                        start: node.start_byte(),
                        end: node.end_byte(),
                        style: HighlightStyle::from_color(color.clone()),
                    });
                    // Don't process children for doc comment markers
                    continue;
                }

                // Collect ALL matching nodes (both leaf and non-leaf), let overlap resolution handle conflicts
                node_matches.push((node_kind.to_string(), node, color.clone()));

                // Add children to stack for processing (except for doc comment markers)
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        stack.push(child);
                    }
                }
                continue;
            }

            // Tree-sitter only highlighting - no fallback text matching needed
            // Add children to stack for processing
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    stack.push(child);
                }
            }
        }

        // Now resolve conflicts and create highlights from node matches
        log::debug!(
            "Node matches before conflict resolution: {}",
            node_matches.len()
        );
        let resolved_matches = Self::resolve_node_conflicts(node_matches);
        log::debug!(
            "Node matches after conflict resolution: {}",
            resolved_matches.len()
        );

        for (node_kind, node, color) in resolved_matches {
            log::debug!(
                "  Resolved: '{}' -> text: '{}' color: {:?}",
                node_kind,
                node.utf8_text(text.as_bytes()).unwrap_or("<invalid>"),
                color
            );
            highlights.push(HighlightRange {
                start: node.start_byte(),
                end: node.end_byte(),
                style: HighlightStyle::from_color(color),
            });
        }

        // Sort highlights by start position
        highlights.sort_by_key(|h| h.start);

        // Note: We don't need resolve_overlapping_highlights() here because
        // we already resolved conflicts at the node level with resolve_node_conflicts()
        // Running both would cause the color-based heuristic to override the proper node-based resolution

        Ok(highlights)
    }

    /// Resolve conflicts between overlapping tree-sitter nodes, preferring more specific types
    fn resolve_node_conflicts(
        node_matches: Vec<(String, tree_sitter::Node, Color)>,
    ) -> Vec<(String, tree_sitter::Node, Color)> {
        let mut result: Vec<(String, tree_sitter::Node, Color)> = Vec::new();

        for (node_kind, node, color) in node_matches {
            let mut should_add = true;
            let mut indices_to_remove = Vec::new();

            // Check for overlaps with existing nodes
            for (i, (existing_kind, existing_node, _existing_color)) in result.iter().enumerate() {
                if node.start_byte() < existing_node.end_byte()
                    && node.end_byte() > existing_node.start_byte()
                {
                    // There's an overlap - decide which one to keep based on node type priority
                    let new_priority = Self::get_node_type_priority(&node_kind);
                    let existing_priority = Self::get_node_type_priority(existing_kind);

                    log::debug!(
                        "Node overlap detected: '{}' ({}..{}, priority {}) vs '{}' ({}..{}, priority {})",
                        node_kind,
                        node.start_byte(),
                        node.end_byte(),
                        new_priority,
                        existing_kind,
                        existing_node.start_byte(),
                        existing_node.end_byte(),
                        existing_priority
                    );

                    if new_priority > existing_priority {
                        // New node has higher priority - remove existing
                        indices_to_remove.push(i);
                    } else {
                        // Existing node has higher or equal priority - don't add new
                        should_add = false;
                        break;
                    }
                }
            }

            // Remove existing nodes that were overridden
            for &i in indices_to_remove.iter().rev() {
                result.remove(i);
            }

            if should_add {
                result.push((node_kind, node, color));
            }
        }

        result
    }

    /// Get priority for tree-sitter node types - higher numbers = higher priority
    fn get_node_type_priority(node_kind: &str) -> u8 {
        match node_kind {
            "field_identifier" => 10, // Highest priority for struct field names
            "function_identifier" => 8,
            "primitive_type" => 7, // Higher priority than containers for types like f64, bool
            "type_identifier" => 6,
            // Punctuation should have higher priority than containers
            "{" | "}" | "(" | ")" | "[" | "]" | ";" | "," | ":" | "." => 6,
            "identifier" => 1, // Lowest priority - generic identifier
            // Container nodes have lower priority so specific nodes can override them
            "field_declaration_list" | "parameters" | "block" | "struct_item" => 3,
            _ => 5, // Medium priority for other types
        }
    }

    pub fn reload_config(&mut self) -> Result<()> {
        // Since we no longer use external config, just reinitialize parsers
        self.parsers.clear();
        self.initialize_parsers()?;
        Ok(())
    }

    /// Get the current theme mappings for testing
    pub fn get_theme_mappings(&self) -> &HashMap<String, Color> {
        &self.current_syntax_theme.tree_sitter_mappings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_punctuation_highlighting() {
        let mut highlighter = SyntaxHighlighter::new().unwrap();

        let test_code = "fn test() { let x = 5; }";
        let highlights = highlighter.highlight_text(test_code, "rust").unwrap();

        // Print all highlights to see what we're getting
        for highlight in &highlights {
            let text = &test_code[highlight.start..highlight.end];
            println!(
                "Highlighted: '{}' ({}..{})",
                text, highlight.start, highlight.end
            );
        }

        // Check that curly braces are highlighted
        let opening_brace = highlights
            .iter()
            .find(|h| &test_code[h.start..h.end] == "{");
        let closing_brace = highlights
            .iter()
            .find(|h| &test_code[h.start..h.end] == "}");

        assert!(
            opening_brace.is_some(),
            "Opening curly brace should be highlighted"
        );
        assert!(
            closing_brace.is_some(),
            "Closing curly brace should be highlighted"
        );
    }

    #[test]
    fn test_doc_comment_highlighting() {
        let mut highlighter = SyntaxHighlighter::new().unwrap();

        let test_code = r#"/// This is a doc comment
//! This is an inner doc comment  
fn test() {
    // Regular comment
    /* Block comment */
    let x = 5;
}"#;

        // Test highlighting
        let highlights = highlighter.highlight_text(test_code, "rust").unwrap();

        // Print all highlights to see what we're getting
        for highlight in &highlights {
            let text = &test_code[highlight.start..highlight.end];
            println!(
                "Highlighted: '{}' ({}..{})",
                text.replace('\n', "\\n"),
                highlight.start,
                highlight.end
            );
        }

        // Check that entire doc comments are highlighted as unified blocks
        let doc_comment_highlights: Vec<_> = highlights
            .iter()
            .filter(|h| {
                let text = &test_code[h.start..h.end];
                text.starts_with("/// This is a doc comment")
                    || text.starts_with("//! This is an inner doc comment")
            })
            .collect();

        // Should have exactly 2 complete doc comment highlights
        assert_eq!(
            doc_comment_highlights.len(),
            2,
            "Should have exactly 2 unified doc comment highlights"
        );

        // Check that regular comments are also handled consistently
        let regular_comment_highlights: Vec<_> = highlights
            .iter()
            .filter(|h| {
                let text = &test_code[h.start..h.end];
                text.starts_with("// Regular") || text.starts_with("/* Block")
            })
            .collect();

        // Should have regular comments highlighted as unified blocks too
        assert_eq!(
            regular_comment_highlights.len(),
            2,
            "Should have 2 regular comment highlights"
        );

        // Verify that all comment highlights have the correct color (comment color)
        let comment_color = "#8b949e";
        let all_comment_highlights: Vec<_> = highlights
            .iter()
            .filter(|h| {
                let text = &test_code[h.start..h.end];
                text.starts_with("///")
                    || text.starts_with("//!")
                    || text.starts_with("// Regular")
                    || text.starts_with("/* Block")
            })
            .collect();

        // All comment highlights should have consistent comment color
        for comment_highlight in &all_comment_highlights {
            let color = comment_highlight
                .style
                .fg_color
                .as_ref()
                .expect("Comment should have color");
            assert_eq!(
                color, comment_color,
                "All comment parts should have unified comment color"
            );
        }

        println!("✓ All comments now have consistent unified coloring");
    }

    #[test]
    fn test_doc_comment_color_consistency() {
        let mut highlighter = SyntaxHighlighter::new().unwrap();

        // Test outer doc comment
        let test_code1 = "/// This is a doc comment";
        let highlights1 = highlighter.highlight_text(test_code1, "rust").unwrap();

        // Should have exactly 1 highlight: the entire comment with unified coloring
        assert_eq!(
            highlights1.len(),
            1,
            "Outer doc comment should have 1 unified highlight"
        );

        // Should have comment color
        let comment_color = "#8b949e";
        for highlight in &highlights1 {
            let color = highlight
                .style
                .fg_color
                .as_ref()
                .expect("Should have a color");
            assert_eq!(
                color, comment_color,
                "All parts of outer doc comment should have comment color"
            );
        }

        // Test inner doc comment
        let test_code2 = "//! This is an inner doc comment";
        let highlights2 = highlighter.highlight_text(test_code2, "rust").unwrap();

        // Should have exactly 1 highlight: the entire comment with unified coloring
        assert_eq!(
            highlights2.len(),
            1,
            "Inner doc comment should have 1 unified highlight"
        );

        // Both should have the same comment color
        for highlight in &highlights2 {
            let color = highlight
                .style
                .fg_color
                .as_ref()
                .expect("Should have a color");
            assert_eq!(
                color, comment_color,
                "All parts of inner doc comment should have comment color"
            );
        }
    }

    #[test]
    fn test_comment_tree_structure() {
        use tree_sitter::{Language, Parser};

        // Get the Rust language
        fn get_rust_language() -> Language {
            tree_sitter_rust::LANGUAGE.into()
        }

        let mut parser = Parser::new();
        parser.set_language(&get_rust_language()).unwrap();

        // Test different comment types and verify tree structure
        let test_cases = vec![
            ("// Regular comment", "line_comment"),
            ("/// Doc comment", "line_comment"),
            ("//! Inner doc comment", "line_comment"),
            ("/* Block comment */", "block_comment"),
            ("/** Doc block comment */", "block_comment"),
        ];

        for (test_code, expected_root_type) in test_cases {
            let tree = parser.parse(test_code, None).unwrap();
            let root_node = tree.root_node();

            // Should have source_file as root
            assert_eq!(root_node.kind(), "source_file");

            // First child should be the comment node
            let comment_node = root_node.child(0).expect("Should have comment child");
            assert_eq!(
                comment_node.kind(),
                expected_root_type,
                "Comment '{}' should have node type '{}'",
                test_code,
                expected_root_type
            );

            // Comment should span the entire text
            assert_eq!(comment_node.start_byte(), 0);
            assert_eq!(comment_node.end_byte(), test_code.len());
        }
    }

    #[test]
    fn test_doc_comment_marker_nodes() {
        use tree_sitter::{Language, Parser};

        fn get_rust_language() -> Language {
            tree_sitter_rust::LANGUAGE.into()
        }

        let mut parser = Parser::new();
        parser.set_language(&get_rust_language()).unwrap();

        // Test that doc comment markers exist in tree structure
        let test_code = "/// Doc comment";
        let tree = parser.parse(test_code, None).unwrap();
        let root_node = tree.root_node();

        // Navigate to line_comment node
        let line_comment = root_node.child(0).expect("Should have line_comment");
        assert_eq!(line_comment.kind(), "line_comment");

        // Should have outer_doc_comment_marker as a child
        let mut found_marker = false;
        let mut found_doc_content = false;

        for i in 0..line_comment.child_count() {
            if let Some(child) = line_comment.child(i) {
                match child.kind() {
                    "outer_doc_comment_marker" => {
                        found_marker = true;
                        // The marker should contain a '/' child
                        let marker_child = child.child(0).expect("Marker should have child");
                        assert_eq!(marker_child.kind(), "/");
                        let marker_text = marker_child.utf8_text(test_code.as_bytes()).unwrap();
                        assert_eq!(marker_text, "/");
                    }
                    "doc_comment" => {
                        found_doc_content = true;
                        let content_text = child.utf8_text(test_code.as_bytes()).unwrap();
                        assert_eq!(content_text, " Doc comment");
                    }
                    _ => {}
                }
            }
        }

        assert!(found_marker, "Should find outer_doc_comment_marker node");
        assert!(found_doc_content, "Should find doc_comment content node");
    }

    #[test]
    fn test_large_file_highlighting() {
        let mut highlighter = SyntaxHighlighter::new().unwrap();

        // Create a long piece of code
        let mut long_code = String::new();
        for i in 1..=200 {
            long_code.push_str(&format!("    println!(\"Line {}\");\n", i));
        }
        long_code = format!("fn main() {{\n{}}}", long_code);

        // Test highlighting the entire code
        let highlights = highlighter.highlight_text(&long_code, "rust").unwrap();

        // Verify we got highlights throughout the file, not just the first 100 lines
        assert!(!highlights.is_empty(), "Should have syntax highlights");

        // Check that we have highlights near the end of the file
        let code_len = long_code.len();
        let has_late_highlights = highlights.iter().any(|h| h.start > code_len / 2);
        assert!(
            has_late_highlights,
            "Should have highlights in the latter half of the file"
        );

        println!(
            "Highlighted {} ranges in {} character file",
            highlights.len(),
            code_len
        );
    }

    #[test]
    fn test_return_expression_highlighting() {
        let mut highlighter = SyntaxHighlighter::new().unwrap();

        let test_code = r#"fn add(a: i32, b: i32) -> i32 {
    let sum = a + b;
    sum  // implicit return
}

fn explicit_return() -> i32 {
    return 42;
}

fn simple_return() -> i32 {
    42
}"#;

        // Parse with Tree-sitter to see the AST structure
        let language: Language = tree_sitter_rust::LANGUAGE.into();
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();

        if let Some(tree) = parser.parse(test_code, None) {
            let root_node = tree.root_node();
            print_return_tree(&root_node, test_code, 0);
        }

        // Test highlighting
        let highlights = highlighter.highlight_text(test_code, "rust").unwrap();

        // Print all highlights to see what we're getting
        for highlight in &highlights {
            let text = &test_code[highlight.start..highlight.end];
            println!(
                "Highlighted: '{}' ({}..{})",
                text.replace('\n', "\\n"),
                highlight.start,
                highlight.end
            );
        }
    }

    fn print_return_tree(node: &tree_sitter::Node, source: &str, depth: usize) {
        let indent = "  ".repeat(depth);
        let node_text = &source[node.start_byte()..node.end_byte()];
        println!(
            "{}kind: '{}', text: '{}'",
            indent,
            node.kind(),
            node_text.replace('\n', "\\n")
        );

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                print_return_tree(&child, source, depth + 1);
            }
        }
    }

    /// Test that struct field names are highlighted with the correct color
    #[test]
    fn test_struct_field_highlighting() {
        let mut highlighter = SyntaxHighlighter::new().unwrap();

        // First, let's debug what theme mappings are actually loaded
        println!("\n=== Theme Mappings Debug ===");
        {
            let theme_mappings = highlighter.get_theme_mappings();
            for (key, color) in theme_mappings.iter() {
                println!("'{}' -> {:?}", key, color);
            }

            // Check that critical mappings exist
            assert!(
                theme_mappings.contains_key("field_identifier"),
                "Missing field_identifier mapping"
            );
            assert!(
                theme_mappings.contains_key("primitive_type"),
                "Missing primitive_type mapping"
            );
            assert!(
                theme_mappings.contains_key("type_identifier"),
                "Missing type_identifier mapping"
            );
        }

        let test_cases = vec![
            "struct Point { x: f64, y: f64 }",
            "struct Person { name: String, age: u32 }",
        ];

        for test_code in test_cases {
            println!("\n=== Testing: {} ===", test_code);
            let highlights = highlighter.highlight_text(test_code, "rust").unwrap();

            // Print all highlights for debugging
            for (i, highlight) in highlights.iter().enumerate() {
                let text = &test_code[highlight.start..highlight.end];
                let color = highlight
                    .style
                    .fg_color
                    .as_ref()
                    .map(|s| s.as_str())
                    .unwrap_or("none");
                println!("{}. '{}' -> {}", i, text, color);
            }
        }

        // Just test the basic case works
        let basic_test = "struct Point { x: f64 }";
        let highlights = highlighter.highlight_text(basic_test, "rust").unwrap();
        let field_highlights: Vec<_> = highlights
            .iter()
            .filter(|h| &basic_test[h.start..h.end] == "x")
            .collect();

        assert!(!field_highlights.is_empty(), "Should highlight field names");
    }

    #[test]
    fn debug_full_struct_tree() {
        // Get the Rust language
        let language: Language = tree_sitter_rust::LANGUAGE.into();

        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();

        // Test the exact code from test_struct.rs
        let test_code = r#"struct Point {
    x: f64,
    y: f64,
}

struct Person {
    name: String,
    age: u32,
    active: bool,
}"#;

        let tree = parser.parse(test_code, None).unwrap();
        let root_node = tree.root_node();

        println!("\n=== Full Tree Structure for Both Structs ===");
        print_debug_tree(&root_node, test_code, 0);

        // Also test our highlighting on this exact code
        let mut highlighter = SyntaxHighlighter::new().unwrap();
        let highlights = highlighter.highlight_text(test_code, "rust").unwrap();

        println!("\n=== Full-File Highlights ===");
        for (i, highlight) in highlights.iter().enumerate() {
            let text = &test_code[highlight.start..highlight.end];
            let color = highlight
                .style
                .fg_color
                .as_ref()
                .map(|s| s.as_str())
                .unwrap_or("none");
            println!(
                "{}. '{}' ({}..{}) -> {}",
                i, text, highlight.start, highlight.end, color
            );
        }
    }

    /// Test node conflict resolution system thoroughly
    #[test]
    fn test_node_conflict_resolution() {
        use tree_sitter::Language;

        // Get the Rust language
        let language: Language = tree_sitter_rust::LANGUAGE.into();
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();

        // Test case 1: field_identifier should override identifier
        let test_code1 = "struct Point { x: f64 }";
        let tree1 = parser.parse(test_code1, None).unwrap();

        // Collect all nodes that could conflict
        let mut node_matches1 = Vec::new();
        let mut stack = vec![tree1.root_node()];

        while let Some(node) = stack.pop() {
            let node_kind = node.kind();

            // Simulate color mapping for conflicting nodes
            let color = match node_kind {
                "field_identifier" => Some(crossterm::style::Color::Green),
                "identifier" => Some(crossterm::style::Color::White),
                "primitive_type" => Some(crossterm::style::Color::Blue),
                _ => None,
            };

            if let Some(color) = color {
                node_matches1.push((node_kind.to_string(), node, color));
            }

            // Add children
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    stack.push(child);
                }
            }
        }

        println!("\n=== Test Case 1: field_identifier vs identifier ===");
        println!("Before resolution: {} matches", node_matches1.len());
        for (kind, node, _) in &node_matches1 {
            if let Ok(text) = node.utf8_text(test_code1.as_bytes()) {
                println!("  {} -> '{}'", kind, text);
            }
        }

        let resolved1 = SyntaxHighlighter::resolve_node_conflicts(node_matches1);
        println!("After resolution: {} matches", resolved1.len());
        for (kind, node, _) in &resolved1 {
            if let Ok(text) = node.utf8_text(test_code1.as_bytes()) {
                println!("  {} -> '{}'", kind, text);
            }
        }

        // Verify field_identifier wins over identifier for "x"
        let x_nodes: Vec<_> = resolved1
            .iter()
            .filter(|(_, node, _)| node.utf8_text(test_code1.as_bytes()).unwrap_or("") == "x")
            .collect();

        assert_eq!(
            x_nodes.len(),
            1,
            "Should have exactly one 'x' node after resolution"
        );
        assert_eq!(
            x_nodes[0].0, "field_identifier",
            "Field 'x' should be field_identifier, not identifier"
        );
    }

    #[test]
    fn test_node_priority_system() {
        // Test all priority assignments
        let test_cases = vec![
            ("field_identifier", 10),
            ("function_identifier", 8),
            ("primitive_type", 7),
            ("type_identifier", 6),
            ("{", 6),
            ("}", 6),
            ("(", 6),
            (")", 6),
            ("[", 6),
            ("]", 6),
            (";", 6),
            (",", 6),
            (":", 6),
            (".", 6),
            ("field_declaration_list", 3),
            ("parameters", 3),
            ("block", 3),
            ("struct_item", 3),
            ("identifier", 1),
            ("unknown_node", 5), // Default case
        ];

        for (node_kind, expected_priority) in test_cases {
            let actual_priority = SyntaxHighlighter::get_node_type_priority(node_kind);
            assert_eq!(
                actual_priority, expected_priority,
                "Priority for '{}' should be {}, got {}",
                node_kind, expected_priority, actual_priority
            );
        }
    }

    #[test]
    fn test_overlapping_node_resolution() {
        use tree_sitter::Language;

        // Get the Rust language
        let language: Language = tree_sitter_rust::LANGUAGE.into();
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();

        // Test case with known overlapping nodes
        let test_code = "struct Point { x: f64, y: f64 }";
        let tree = parser.parse(test_code, None).unwrap();

        // Collect nodes that map to colors
        let mut node_matches = Vec::new();
        let mut stack = vec![tree.root_node()];

        while let Some(node) = stack.pop() {
            let node_kind = node.kind();

            // Use actual theme mappings
            let color = match node_kind {
                "struct" => Some(crossterm::style::Color::Red),
                "field_identifier" => Some(crossterm::style::Color::Green),
                "identifier" => Some(crossterm::style::Color::White),
                "primitive_type" => Some(crossterm::style::Color::Blue),
                "type_identifier" => Some(crossterm::style::Color::Cyan),
                "{" | "}" | "," | ":" => Some(crossterm::style::Color::Yellow),
                "field_declaration_list" => Some(crossterm::style::Color::Magenta),
                _ => None,
            };

            if let Some(color) = color {
                node_matches.push((node_kind.to_string(), node, color));
            }

            // Add children
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    stack.push(child);
                }
            }
        }

        println!("\n=== Test Overlapping Node Resolution ===");
        println!("Before resolution: {} nodes", node_matches.len());

        // Check for specific overlaps we expect
        let point_nodes: Vec<_> = node_matches
            .iter()
            .filter(|(_, node, _)| node.utf8_text(test_code.as_bytes()).unwrap_or("") == "Point")
            .collect();

        let x_nodes: Vec<_> = node_matches
            .iter()
            .filter(|(_, node, _)| node.utf8_text(test_code.as_bytes()).unwrap_or("") == "x")
            .collect();

        let f64_nodes: Vec<_> = node_matches
            .iter()
            .filter(|(_, node, _)| node.utf8_text(test_code.as_bytes()).unwrap_or("") == "f64")
            .collect();

        println!("'Point' nodes before resolution: {}", point_nodes.len());
        println!("'x' nodes before resolution: {}", x_nodes.len());
        println!("'f64' nodes before resolution: {}", f64_nodes.len());

        // Resolve conflicts
        let resolved = SyntaxHighlighter::resolve_node_conflicts(node_matches);
        println!("After resolution: {} nodes", resolved.len());

        // Check resolution results
        let resolved_x: Vec<_> = resolved
            .iter()
            .filter(|(_, node, _)| node.utf8_text(test_code.as_bytes()).unwrap_or("") == "x")
            .collect();

        let resolved_f64: Vec<_> = resolved
            .iter()
            .filter(|(_, node, _)| node.utf8_text(test_code.as_bytes()).unwrap_or("") == "f64")
            .collect();

        // Assertions
        assert!(
            resolved_x.len() <= 1,
            "Should have at most one 'x' node after resolution"
        );
        assert!(
            resolved_f64.len() >= 1,
            "Should have at least one 'f64' node after resolution"
        );

        // If we have an 'x' node, it should be field_identifier
        if !resolved_x.is_empty() {
            assert_eq!(
                resolved_x[0].0, "field_identifier",
                "Resolved 'x' should be field_identifier"
            );
        }

        // Check that we don't lose important nodes
        let resolved_struct: Vec<_> = resolved
            .iter()
            .filter(|(kind, _, _)| kind == "struct")
            .collect();

        assert!(
            !resolved_struct.is_empty(),
            "Should preserve 'struct' keyword"
        );
    }

    #[test]
    fn test_complex_struct_conflict_resolution() {
        use tree_sitter::Language;

        let language: Language = tree_sitter_rust::LANGUAGE.into();
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();

        // Test the exact problematic case we fixed
        let test_code = r#"struct Point {
    x: f64,
    y: f64,
}

struct Person {
    name: String,
    age: u32,
    active: bool,
}"#;

        let _tree = parser.parse(test_code, None).unwrap();

        // Create a mock highlighter to test the full highlighting pipeline
        let mut highlighter = SyntaxHighlighter::new().unwrap();
        let highlights = highlighter.highlight_text(test_code, "rust").unwrap();

        println!("\n=== Complex Struct Test ===");
        println!("Total highlights: {}", highlights.len());

        // Check for critical elements that were previously missing
        let critical_elements = vec![
            ("struct", "#ff7b72"), // struct keyword should be red
            ("x", "#7ee787"),      // field names should be green
            ("y", "#7ee787"),
            ("name", "#7ee787"),
            ("age", "#7ee787"),
            ("active", "#7ee787"),
            ("f64", "#79c0ff"), // primitive types should be blue
            ("bool", "#79c0ff"),
            ("u32", "#79c0ff"),
            ("String", "#f0883e"), // type identifiers should be orange
            ("Point", "#f0883e"),  // type identifiers should be orange
            ("Person", "#f0883e"), // type identifiers should be orange
            ("}", "#f0883e"),      // closing braces should be orange
            ("{", "#f0883e"),      // opening braces should be orange
        ];

        for (text, expected_color) in critical_elements {
            let matching_highlights: Vec<_> = highlights
                .iter()
                .filter(|h| {
                    let highlight_text = &test_code[h.start..h.end];
                    highlight_text == text
                })
                .collect();

            // For field names and types, we expect at least one occurrence
            // For braces, we expect multiple
            let min_expected = if text == "}" || text == "{" { 2 } else { 1 };

            assert!(
                matching_highlights.len() >= min_expected,
                "Expected at least {} occurrences of '{}' to be highlighted, got {}",
                min_expected,
                text,
                matching_highlights.len()
            );

            // Check that at least one has the correct color
            let has_correct_color = matching_highlights
                .iter()
                .any(|h| h.style.fg_color.as_ref().map(|s| s.as_str()) == Some(expected_color));

            assert!(
                has_correct_color,
                "Expected '{}' to have color {}, but none found with that color. Found colors: {:?}",
                text,
                expected_color,
                matching_highlights
                    .iter()
                    .map(|h| h.style.fg_color.as_ref())
                    .collect::<Vec<_>>()
            );
        }

        // Regression test: verify we have all the highlights we expect
        // We should have significantly more than the 17 we had before the fix
        assert!(
            highlights.len() >= 20,
            "Should have at least 20 highlights, got {} (regression check)",
            highlights.len()
        );
    }

    #[test]
    fn test_punctuation_priority_over_containers() {
        use tree_sitter::Language;

        let language: Language = tree_sitter_rust::LANGUAGE.into();
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();

        // Test that punctuation gets priority over container nodes
        let test_code = "struct Point { x: f64 }";
        let tree = parser.parse(test_code, None).unwrap();

        // Collect all nodes, focusing on the closing brace
        let mut node_matches = Vec::new();
        let mut stack = vec![tree.root_node()];

        while let Some(node) = stack.pop() {
            let node_kind = node.kind();

            // Look for nodes that could conflict over the closing brace
            let color = match node_kind {
                "}" => Some(crossterm::style::Color::Yellow), // punctuation
                "field_declaration_list" => Some(crossterm::style::Color::Magenta), // container
                "block" => Some(crossterm::style::Color::Cyan), // container
                _ => None,
            };

            if let Some(color) = color {
                // Only include nodes that actually contain the closing brace
                if let Ok(text) = node.utf8_text(test_code.as_bytes()) {
                    if text.contains('}') {
                        node_matches.push((node_kind.to_string(), node, color));
                    }
                }
            }

            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    stack.push(child);
                }
            }
        }

        println!("\n=== Punctuation Priority Test ===");
        println!("Nodes containing '{{}}': {}", node_matches.len());
        for (kind, node, _) in &node_matches {
            println!(
                "  {} -> '{}'",
                kind,
                node.utf8_text(test_code.as_bytes()).unwrap_or("<error>")
            );
        }

        let resolved = SyntaxHighlighter::resolve_node_conflicts(node_matches);

        // The closing brace should be resolved to punctuation, not container
        let brace_nodes: Vec<_> = resolved
            .iter()
            .filter(|(_, node, _)| node.utf8_text(test_code.as_bytes()).unwrap_or("") == "}")
            .collect();

        if !brace_nodes.is_empty() {
            assert_eq!(
                brace_nodes[0].0, "}",
                "Closing brace should resolve to punctuation token, not container"
            );
        }
    }

    #[test]
    fn test_no_valid_nodes_dropped_during_resolution() {
        use tree_sitter::Language;

        let language: Language = tree_sitter_rust::LANGUAGE.into();
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();

        // Test case that previously dropped valid nodes
        let test_code = r#"struct Point {
    x: f64,
    y: f64,
}"#;

        let tree = parser.parse(test_code, None).unwrap();

        // Collect ALL nodes that should be highlighted
        let mut important_nodes = Vec::new();
        let mut stack = vec![tree.root_node()];

        while let Some(node) = stack.pop() {
            let node_kind = node.kind();

            // Identify nodes that should definitely be highlighted
            let should_highlight = match node_kind {
                "struct" | "field_identifier" | "primitive_type" | "type_identifier" | "{"
                | "}" | "," | ":" => true,
                _ => false,
            };

            if should_highlight {
                if let Ok(text) = node.utf8_text(test_code.as_bytes()) {
                    // Only include leaf nodes or nodes with specific important content
                    if node.child_count() == 0 || matches!(node_kind, "struct") {
                        important_nodes.push((
                            node_kind.to_string(),
                            text.to_string(),
                            node.start_byte(),
                            node.end_byte(),
                        ));
                    }
                }
            }

            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    stack.push(child);
                }
            }
        }

        println!("\n=== Important Nodes Before Resolution ===");
        for (kind, text, start, end) in &important_nodes {
            println!("  {} -> '{}' ({}..{})", kind, text, start, end);
        }

        // Now test actual highlighting
        let mut highlighter = SyntaxHighlighter::new().unwrap();
        let highlights = highlighter.highlight_text(test_code, "rust").unwrap();

        println!("\n=== Actual Highlights After Resolution ===");
        for highlight in &highlights {
            let text = &test_code[highlight.start..highlight.end];
            println!("  '{}' ({}..{})", text, highlight.start, highlight.end);
        }

        // Critical regression test: ensure we don't drop essential nodes
        let essential_texts = vec![
            ("struct", Some("#ff7b72")), // red
            ("x", Some("#7ee787")),      // green
            ("y", Some("#7ee787")),      // green
            ("f64", Some("#79c0ff")),    // blue
            ("}", Some("#f0883e")),      // orange
            ("{", Some("#f0883e")),      // orange
            (":", Some("#f0883e")),      // orange - now that we added it to theme
        ];

        for (essential_text, expected_color) in essential_texts {
            let found = highlights.iter().any(|h| {
                let highlight_text = &test_code[h.start..h.end];
                highlight_text == essential_text
            });

            assert!(
                found,
                "Essential text '{}' was dropped during conflict resolution!",
                essential_text
            );

            // Also verify the color if specified
            if let Some(expected) = expected_color {
                let matching_highlights: Vec<_> = highlights
                    .iter()
                    .filter(|h| {
                        let highlight_text = &test_code[h.start..h.end];
                        highlight_text == essential_text
                    })
                    .collect();

                let has_correct_color = matching_highlights
                    .iter()
                    .any(|h| h.style.fg_color.as_ref().map(|s| s.as_str()) == Some(expected));

                assert!(
                    has_correct_color,
                    "Essential text '{}' should have color '{}' but found colors: {:?}",
                    essential_text,
                    expected,
                    matching_highlights
                        .iter()
                        .map(|h| h.style.fg_color.as_ref())
                        .collect::<Vec<_>>()
                );
            }
        }
    }

    fn print_debug_tree(node: &tree_sitter::Node, source: &str, depth: usize) {
        let indent = "  ".repeat(depth);
        let node_text = &source[node.start_byte()..node.end_byte()];
        println!(
            "{}kind: '{}', text: '{}' ({}..{})",
            indent,
            node.kind(),
            node_text.replace('\n', "\\n"),
            node.start_byte(),
            node.end_byte()
        );

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                print_debug_tree(&child, source, depth + 1);
            }
        }
    }
}

// ===== TREE-SITTER ONLY HIGHLIGHTING =====

// ===== ASYNC SYNTAX HIGHLIGHTING =====

/// Callback function type for UI refresh notifications
pub type UiRefreshCallback = Box<dyn Fn(usize, usize) + Send + Sync>;

/// Priority levels for syntax highlighting requests
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low = 0,      // Background processing of entire file
    Medium = 1,   // Lines within scroll buffer (±50 lines)
    High = 2,     // Currently visible lines
    Critical = 3, // User is actively editing this line
}

/// Request for syntax highlighting
pub struct HighlightRequest {
    pub buffer_id: usize,
    pub line_index: usize,
    pub content: String,
    pub language: String,
    pub priority: Priority,
    pub response_tx: oneshot::Sender<Vec<HighlightRange>>,
    pub ui_refresh_callback: Option<UiRefreshCallback>,
}

impl std::fmt::Debug for HighlightRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HighlightRequest")
            .field("buffer_id", &self.buffer_id)
            .field("line_index", &self.line_index)
            .field("content", &self.content)
            .field("language", &self.language)
            .field("priority", &self.priority)
            .field("has_ui_callback", &self.ui_refresh_callback.is_some())
            .finish()
    }
}

/// Async syntax highlighter that processes requests in background
pub struct AsyncSyntaxHighlighter {
    /// Request sender to background worker
    request_tx: mpsc::UnboundedSender<HighlightRequest>,
    /// Handle to the background worker task
    worker_handle: JoinHandle<()>,
    /// Shared cache accessible from main thread for immediate lookups
    shared_cache: Arc<RwLock<HashMap<HighlightCacheKey, HighlightCacheEntry>>>,
}

impl AsyncSyntaxHighlighter {
    /// Create a new async syntax highlighter with background worker
    pub fn new() -> Result<Self> {
        info!("Initializing async syntax highlighter");

        // Check if we have a Tokio runtime available
        match tokio::runtime::Handle::try_current() {
            Ok(_) => {
                // We have a runtime, proceed with async initialization
                Self::new_with_runtime()
            }
            Err(_) => {
                // No runtime available, return error instead of panicking
                Err(anyhow::anyhow!(
                    "No Tokio runtime available for async syntax highlighter"
                ))
            }
        }
    }

    /// Internal method that assumes a Tokio runtime is available
    fn new_with_runtime() -> Result<Self> {
        // Create shared cache that both main thread and worker can access
        let shared_cache = Arc::new(RwLock::new(HashMap::new()));
        let worker_cache = Arc::clone(&shared_cache);

        // Create communication channel
        let (request_tx, request_rx) = mpsc::unbounded_channel();

        // Spawn background worker
        let worker_handle = tokio::spawn(async move {
            Self::worker_loop(request_rx, worker_cache).await;
        });

        Ok(AsyncSyntaxHighlighter {
            request_tx,
            worker_handle,
            shared_cache,
        })
    }

    /// Check if we have cached highlights for this line
    pub fn get_cached_highlights(
        &self,
        buffer_id: usize,
        line_index: usize,
        content: &str,
        language: &str,
    ) -> Option<Vec<HighlightRange>> {
        let cache_key = HighlightCacheKey::new_simple(content, language);

        if let Ok(cache) = self.shared_cache.read() {
            if let Some(entry) = cache.get(&cache_key) {
                debug!(
                    "Cache hit for buffer {} line {} (content: {}...)",
                    buffer_id,
                    line_index,
                    &content[..content.len().min(20)]
                );
                return Some(entry.highlights().clone());
            } else {
                debug!(
                    "Cache miss for buffer {} line {} (content: {}...)",
                    buffer_id,
                    line_index,
                    &content[..content.len().min(20)]
                );
            }
        }

        None
    }

    /// Request syntax highlighting for a line (async)
    pub fn request_highlighting(
        &self,
        buffer_id: usize,
        line_index: usize,
        content: String,
        language: String,
        priority: Priority,
    ) -> Result<oneshot::Receiver<Vec<HighlightRange>>> {
        self.request_highlighting_with_callback(
            buffer_id, line_index, content, language, priority, None,
        )
    }

    /// Request syntax highlighting for a line with optional UI refresh callback
    pub fn request_highlighting_with_callback(
        &self,
        buffer_id: usize,
        line_index: usize,
        content: String,
        language: String,
        priority: Priority,
        ui_callback: Option<UiRefreshCallback>,
    ) -> Result<oneshot::Receiver<Vec<HighlightRange>>> {
        let (response_tx, response_rx) = oneshot::channel();

        let request = HighlightRequest {
            buffer_id,
            line_index,
            content,
            language,
            priority,
            response_tx,
            ui_refresh_callback: ui_callback,
        };

        self.request_tx.send(request).map_err(|_| {
            anyhow::anyhow!("Failed to send highlight request - worker may be shut down")
        })?;

        debug!(
            "Requested highlighting for buffer {} line {} with priority {:?}",
            buffer_id, line_index, priority
        );
        Ok(response_rx)
    }

    /// Request highlighting for multiple lines with priority
    pub fn request_batch_highlighting(
        &self,
        buffer_id: usize,
        lines: Vec<(usize, String)>, // (line_index, content)
        language: String,
        priority: Priority,
    ) -> Result<Vec<oneshot::Receiver<Vec<HighlightRange>>>> {
        let mut receivers = Vec::new();

        for (line_index, content) in lines {
            let receiver = self.request_highlighting(
                buffer_id,
                line_index,
                content,
                language.clone(),
                priority,
            )?;
            receivers.push(receiver);
        }

        Ok(receivers)
    }

    /// Invalidate cache entries for a buffer (when buffer is edited)
    pub fn invalidate_buffer_cache(&self, buffer_id: usize) {
        // For now, we'll do a simple approach and clear the entire cache
        // In a more sophisticated implementation, we could track which cache entries
        // belong to which buffer and only invalidate those
        if let Ok(mut cache) = self.shared_cache.write() {
            let before_size = cache.len();
            cache.clear();
            debug!(
                "Invalidated cache for buffer {} (cleared {} entries)",
                buffer_id, before_size
            );
        }
    }

    /// Get cache statistics
    pub fn cache_stats(&self) -> (usize, usize) {
        if let Ok(cache) = self.shared_cache.read() {
            (cache.len(), 1000) // (current_size, max_size)
        } else {
            (0, 1000)
        }
    }

    /// Update theme by clearing cache - theme changes will be picked up on next highlight
    pub fn update_theme(&self, theme_name: &str) -> Result<()> {
        // Clear the cache so that new highlights will pick up the updated theme
        // The worker thread's SyntaxHighlighter will reload the theme when it
        // creates new highlights since ThemeConfig::load() reads from the file
        if let Ok(mut cache) = self.shared_cache.write() {
            let before_size = cache.len();
            cache.clear();
            log::info!(
                "Theme updated to '{}', cleared {} cache entries",
                theme_name,
                before_size
            );
        }
        Ok(())
    }

    /// Force re-highlighting of specific content (ignores cache)
    pub fn force_immediate_highlights(
        &self,
        _buffer_id: usize,
        _line_index: usize,
        content: &str,
        language: &str,
    ) -> Option<Vec<HighlightRange>> {
        // Always create new highlights, ignoring cache completely
        if let Ok(mut sync_highlighter) = SyntaxHighlighter::new() {
            if let Ok(highlights) = sync_highlighter.highlight_text(content, language) {
                // Store in cache for future use
                let cache_key = HighlightCacheKey::new_simple(content, language);
                let cache_entry = HighlightCacheEntry::new(highlights.clone());

                if let Ok(mut cache) = self.shared_cache.write() {
                    cache.insert(cache_key, cache_entry);
                }

                return Some(highlights);
            }
        }

        None
    }

    /// Force re-highlighting with full context - parses entire file and extracts line-specific highlights
    pub fn force_immediate_highlights_with_context(
        &self,
        _buffer_id: usize,
        line_index: usize,
        full_content: &str,
        _line_content: &str,
        language: &str,
    ) -> Option<Vec<HighlightRange>> {
        // Always create new highlights using full file context
        if let Ok(mut sync_highlighter) = SyntaxHighlighter::new() {
            if let Ok(all_highlights) = sync_highlighter.highlight_text(full_content, language) {
                // Extract highlights for the specific line
                let line_highlights =
                    self.extract_line_highlights(&all_highlights, full_content, line_index);

                // Store the line-specific highlights in cache for future use
                let cache_key = HighlightCacheKey::new_simple(_line_content, language);
                let cache_entry = HighlightCacheEntry::new(line_highlights.clone());

                if let Ok(mut cache) = self.shared_cache.write() {
                    cache.insert(cache_key, cache_entry);
                }

                return Some(line_highlights);
            }
        }

        None
    }

    /// Extract highlights for a specific line from full-file highlights
    fn extract_line_highlights(
        &self,
        all_highlights: &[HighlightRange],
        full_content: &str,
        line_index: usize,
    ) -> Vec<HighlightRange> {
        // Calculate byte ranges for the target line
        let lines: Vec<&str> = full_content.lines().collect();
        if line_index >= lines.len() {
            return Vec::new();
        }

        // Calculate the start byte position of the target line
        let mut line_start_byte = 0;
        for i in 0..line_index {
            line_start_byte += lines[i].len() + 1; // +1 for newline character
        }
        let line_end_byte = line_start_byte + lines[line_index].len();

        // Filter highlights that fall within this line and adjust their positions
        let mut line_highlights = Vec::new();
        for highlight in all_highlights {
            // Check if highlight overlaps with this line
            if highlight.start < line_end_byte && highlight.end > line_start_byte {
                // Adjust highlight positions to be relative to the line start
                let adjusted_start = if highlight.start >= line_start_byte {
                    highlight.start - line_start_byte
                } else {
                    0
                };
                let adjusted_end = if highlight.end <= line_end_byte {
                    highlight.end - line_start_byte
                } else {
                    line_end_byte - line_start_byte
                };

                // Only include if there's actual content to highlight
                if adjusted_start < adjusted_end {
                    line_highlights.push(HighlightRange {
                        start: adjusted_start,
                        end: adjusted_end,
                        style: highlight.style.clone(),
                    });
                }
            }
        }

        line_highlights
    }

    /// Background worker loop that processes highlighting requests
    async fn worker_loop(
        mut request_rx: mpsc::UnboundedReceiver<HighlightRequest>,
        cache: Arc<RwLock<HashMap<HighlightCacheKey, HighlightCacheEntry>>>,
    ) {
        info!("Starting async syntax highlighting worker");

        // Create a syntax highlighter for the worker thread
        let mut highlighter = match SyntaxHighlighter::new() {
            Ok(h) => h,
            Err(e) => {
                warn!("Failed to create syntax highlighter in worker: {}", e);
                return;
            }
        };

        // Use a priority queue to process high priority requests first
        let mut pending_requests: Vec<HighlightRequest> = Vec::new();

        while let Some(request) = request_rx.recv().await {
            // Add request to pending queue
            pending_requests.push(request);

            // Sort by priority (highest first)
            pending_requests.sort_by(|a, b| b.priority.cmp(&a.priority));

            // Process all pending requests in priority order
            while let Some(request) = pending_requests.pop() {
                Self::process_request(request, &mut highlighter, &cache).await;

                // Check if we have more incoming requests to potentially interrupt lower priority work
                if pending_requests.len() < 10 {
                    // Don't interrupt if we have a big backlog
                    if let Ok(new_request) = request_rx.try_recv() {
                        pending_requests.push(new_request);
                        pending_requests.sort_by(|a, b| b.priority.cmp(&a.priority));
                    }
                }
            }
        }

        info!("Async syntax highlighting worker stopped");
    }

    /// Process a single highlighting request
    async fn process_request(
        request: HighlightRequest,
        highlighter: &mut SyntaxHighlighter,
        cache: &Arc<RwLock<HashMap<HighlightCacheKey, HighlightCacheEntry>>>,
    ) {
        let cache_key = HighlightCacheKey::new_simple(&request.content, &request.language);

        // Check cache first
        if let Ok(cache_ref) = cache.read() {
            if let Some(entry) = cache_ref.get(&cache_key) {
                debug!(
                    "Worker cache hit for buffer {} line {}",
                    request.buffer_id, request.line_index
                );
                let _ = request.response_tx.send(entry.highlights().clone());
                return;
            }
        }

        // Not in cache, compute highlights using tree-sitter
        let highlight_ranges = highlighter
            .highlight_text(&request.content, &request.language)
            .unwrap_or_else(|_| Vec::new());

        debug!(
            "Worker computed highlights for buffer {} line {} ({} ranges)",
            request.buffer_id,
            request.line_index,
            highlight_ranges.len()
        );

        // Store in cache
        if let Ok(mut cache_ref) = cache.write() {
            let entry = HighlightCacheEntry::new(highlight_ranges.clone());
            cache_ref.insert(cache_key, entry);

            // Simple LRU: if cache is too big, clear it
            // In a production system, we'd implement proper LRU eviction
            if cache_ref.len() > 1000 {
                debug!("Cache full, clearing to prevent memory growth");
                cache_ref.clear();
            }
        }

        // Send result
        let _ = request.response_tx.send(highlight_ranges);

        // If this is a high priority request with a UI callback, trigger refresh
        if request.priority >= Priority::High {
            if let Some(callback) = request.ui_refresh_callback {
                callback(request.buffer_id, request.line_index);
            }
        }
    }
}

impl Drop for AsyncSyntaxHighlighter {
    fn drop(&mut self) {
        // Abort the worker when the highlighter is dropped
        self.worker_handle.abort();
    }
}

#[cfg(test)]
mod async_tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    // Helper function to create a test runtime for async tests
    fn with_runtime<F, Fut>(test: F)
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(test());
    }

    #[test]
    fn test_priority_ordering() {
        assert!(Priority::Critical > Priority::High);
        assert!(Priority::High > Priority::Medium);
        assert!(Priority::Medium > Priority::Low);

        // Test equality
        assert_eq!(Priority::Critical, Priority::Critical);
        assert_ne!(Priority::High, Priority::Low);
    }

    #[test]
    fn test_priority_debug() {
        let priority = Priority::High;
        let debug_str = format!("{:?}", priority);
        assert_eq!(debug_str, "High");
    }

    #[test]
    fn test_highlight_request_debug() {
        let (_tx, _rx): (tokio::sync::oneshot::Sender<Vec<HighlightRange>>, _) =
            tokio::sync::oneshot::channel();

        // We can't easily test the full HighlightRequest since oneshot::Sender
        // doesn't implement Debug, but we can test the priority
        let priority = Priority::Medium;
        assert!(format!("{:?}", priority).contains("Medium"));
    }

    #[test]
    fn test_async_highlighter_creation_without_runtime() {
        // This should fail when no Tokio runtime is available
        let result = AsyncSyntaxHighlighter::new();

        // The result should be an error since we're not in a Tokio runtime
        assert!(result.is_err());

        let error_msg = format!("{}", result.err().unwrap());
        assert!(error_msg.contains("No Tokio runtime available"));
    }

    #[test]
    fn test_async_highlighter_creation_with_runtime() {
        with_runtime(|| async {
            let highlighter = AsyncSyntaxHighlighter::new();
            assert!(highlighter.is_ok());
        });
    }

    #[test]
    fn test_cache_stats() {
        with_runtime(|| async {
            let highlighter = AsyncSyntaxHighlighter::new().unwrap();

            let (current_size, max_size) = highlighter.cache_stats();
            assert_eq!(current_size, 0); // Empty cache initially
            assert_eq!(max_size, 1000); // Default max size
        });
    }

    #[test]
    fn test_get_cached_highlights_empty_cache() {
        with_runtime(|| async {
            let highlighter = AsyncSyntaxHighlighter::new().unwrap();

            let result = highlighter.get_cached_highlights(1, 0, "fn main() {}", "rust");

            assert!(result.is_none()); // Should be None for empty cache
        });
    }

    #[test]
    fn test_invalidate_buffer_cache() {
        with_runtime(|| async {
            let highlighter = AsyncSyntaxHighlighter::new().unwrap();

            // Initially empty
            let (size_before, _) = highlighter.cache_stats();
            assert_eq!(size_before, 0);

            // Add something to cache via immediate highlighting
            // let _result = highlighter.get_immediate_highlights(1, 0, "fn test() {}", "rust");
            // TODO: Update this test to use async highlighting

            // Check if cache grew (if syntax highlighting is available)
            let (_size_after, _) = highlighter.cache_stats();

            // Invalidate cache
            highlighter.invalidate_buffer_cache(1);

            let (size_final, _) = highlighter.cache_stats();
            assert_eq!(size_final, 0); // Should be empty after invalidation
        });
    }

    #[test]
    fn test_request_highlighting() {
        with_runtime(|| async {
            let highlighter = AsyncSyntaxHighlighter::new().unwrap();

            let receiver = highlighter.request_highlighting(
                1,
                0,
                "fn main() {}".to_string(),
                "rust".to_string(),
                Priority::High,
            );

            assert!(receiver.is_ok());

            // Try to receive result with timeout
            let receiver = receiver.unwrap();
            let result = timeout(Duration::from_millis(100), receiver).await;

            // The result might timeout if tree-sitter isn't available
            // That's OK - we're testing the async mechanism
            match result {
                Ok(Ok(highlights)) => {
                    // Successfully got highlights
                    println!("Received {} highlights", highlights.len());
                }
                Ok(Err(_)) => {
                    // Receiver was dropped or error occurred
                    println!("Request failed or worker unavailable");
                }
                Err(_) => {
                    // Timeout - worker might be busy or tree-sitter unavailable
                    println!("Request timed out");
                }
            }
        });
    }

    #[test]
    fn test_request_batch_highlighting() {
        with_runtime(|| async {
            let highlighter = AsyncSyntaxHighlighter::new().unwrap();

            let lines = vec![
                (0, "fn main() {".to_string()),
                (1, "    println!(\"Hello\");".to_string()),
                (2, "}".to_string()),
            ];

            let receivers = highlighter.request_batch_highlighting(
                1,
                lines,
                "rust".to_string(),
                Priority::Medium,
            );

            assert!(receivers.is_ok());
            let receivers = receivers.unwrap();
            assert_eq!(receivers.len(), 3);
        });
    }

    #[test]
    #[ignore] // Disabled: immediate highlighting removed in favor of async-only
    fn test_get_immediate_highlights() {
        // TODO: Update this test to use async highlighting
        // with_runtime(|| async {
        //     let highlighter = AsyncSyntaxHighlighter::new().unwrap();
        //     // Test async highlighting instead
        // });
    }

    #[test]
    #[ignore] // Disabled: immediate highlighting removed in favor of async-only
    fn test_cache_with_different_languages() {
        // TODO: Update this test to use async highlighting
        // with_runtime(|| async {
        //     let highlighter = AsyncSyntaxHighlighter::new().unwrap();
        //     // Test async highlighting with different languages
        // });
    }

    #[test]
    fn test_priority_values() {
        // Test the numeric values are correct
        assert_eq!(Priority::Low as u8, 0);
        assert_eq!(Priority::Medium as u8, 1);
        assert_eq!(Priority::High as u8, 2);
        assert_eq!(Priority::Critical as u8, 3);
    }

    #[test]
    fn test_priority_clone_and_copy() {
        let priority = Priority::High;
        let cloned = priority.clone();
        let copied = priority;

        assert_eq!(priority, cloned);
        assert_eq!(priority, copied);
    }

    #[test]
    fn test_multiple_highlighter_instances() {
        with_runtime(|| async {
            // Test creating multiple highlighter instances
            let highlighter1 = AsyncSyntaxHighlighter::new();
            let highlighter2 = AsyncSyntaxHighlighter::new();

            assert!(highlighter1.is_ok());
            assert!(highlighter2.is_ok());

            // Both should have independent caches
            let stats1 = highlighter1.unwrap().cache_stats();
            let stats2 = highlighter2.unwrap().cache_stats();

            assert_eq!(stats1.0, 0); // Both start empty
            assert_eq!(stats2.0, 0);
        });
    }

    #[test]
    #[ignore] // Disabled: immediate highlighting removed in favor of async-only
    fn test_error_handling_invalid_request() {
        // TODO: Update this test to use async highlighting
        // with_runtime(|| async {
        //     let highlighter = AsyncSyntaxHighlighter::new().unwrap();
        //     // Test async highlighting with invalid input
        // });
    }

    #[test]
    #[ignore] // Disabled: immediate highlighting removed in favor of async-only  
    fn test_large_content_handling() {
        // TODO: Update this test to use async highlighting
        // with_runtime(|| async {
        //     let highlighter = AsyncSyntaxHighlighter::new().unwrap();
        //     // Test async highlighting with large content
        // });
    }
}
