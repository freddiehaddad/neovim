use anyhow::{Result, anyhow};
use crossterm::style::Color;
use log::{debug, info, trace};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use tree_sitter::{Language, Parser};

use crate::theme::{SyntaxTheme, ThemeConfig};

/// Check if a node is an implicit return value (last expression in a function block)
fn is_implicit_return(node: &tree_sitter::Node, block_parent: &tree_sitter::Node) -> bool {
    // Check if the block is part of a function
    if let Some(function_parent) = block_parent.parent() {
        if function_parent.kind() != "function_item" {
            return false;
        }
    } else {
        return false;
    }

    // Find the last meaningful child of the block (excluding closing brace)
    let mut last_meaningful_child = None;
    for i in 0..block_parent.child_count() {
        if let Some(child) = block_parent.child(i) {
            // Skip punctuation like opening/closing braces
            if !matches!(child.kind(), "{" | "}") {
                last_meaningful_child = Some(child);
            }
        }
    }

    // Check if this node is the last meaningful child
    if let Some(last_child) = last_meaningful_child {
        last_child.id() == node.id()
    } else {
        false
    }
}

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
        Self::new(content, language, "default")
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
        let extension = Path::new(file_path).extension()?.to_str()?;

        // Hardcoded language detection for Rust
        if extension == "rs" {
            Some("rust".to_string())
        } else {
            None
        }
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

        // Use a simplified approach - query all nodes manually using the Tree-sitter tree
        let mut stack = vec![tree.root_node()];

        while let Some(node) = stack.pop() {
            let node_kind = node.kind();

            // Check for Tree-sitter node types and apply our new theme colors
            match node_kind {
                "string_literal" | "raw_string_literal" | "char_literal" => {
                    highlights.push(HighlightRange {
                        start: node.start_byte(),
                        end: node.end_byte(),
                        style: HighlightStyle::from_color(self.current_syntax_theme.string.clone()),
                    });
                }
                "line_comment" | "block_comment" => {
                    highlights.push(HighlightRange {
                        start: node.start_byte(),
                        end: node.end_byte(),
                        style: HighlightStyle::from_color(
                            self.current_syntax_theme.comment.clone(),
                        ),
                    });

                    // Skip processing children for comments to avoid highlighting
                    // individual comment markers (like the third '/' in doc comments)
                    continue;
                }
                "integer_literal" | "float_literal" => {
                    highlights.push(HighlightRange {
                        start: node.start_byte(),
                        end: node.end_byte(),
                        style: HighlightStyle::from_color(self.current_syntax_theme.number.clone()),
                    });
                }
                "boolean_literal" => {
                    highlights.push(HighlightRange {
                        start: node.start_byte(),
                        end: node.end_byte(),
                        style: HighlightStyle::from_color(
                            self.current_syntax_theme.boolean.clone(),
                        ),
                    });
                }
                "macro_invocation" => {
                    // For macro invocations, highlight only the macro name (first child), not the entire call
                    if let Some(macro_name_node) = node.child(0) {
                        highlights.push(HighlightRange {
                            start: macro_name_node.start_byte(),
                            end: macro_name_node.end_byte(),
                            style: HighlightStyle::from_color(
                                self.current_syntax_theme.macro_color.clone(),
                            ),
                        });
                    }

                    // Add children to stack, but skip the first child (macro name) since we already highlighted it
                    for i in 1..node.child_count() {
                        if let Some(child) = node.child(i) {
                            stack.push(child);
                        }
                    }
                    continue; // Skip the normal child processing for this node
                }
                "identifier" => {
                    // Skip identifiers that are part of macro invocations (already highlighted)
                    if let Some(parent) = node.parent() {
                        if parent.kind() == "macro_invocation"
                            && parent.child(0).map(|c| c.id()) == Some(node.id())
                        {
                            // This is the macro name, already highlighted above
                            continue;
                        }

                        // Check if this identifier is an implicit return value
                        if parent.kind() == "block" && is_implicit_return(&node, &parent) {
                            highlights.push(HighlightRange {
                                start: node.start_byte(),
                                end: node.end_byte(),
                                style: HighlightStyle::from_color(
                                    self.current_syntax_theme.function.clone(), // Use function color for return values
                                ),
                            });
                            continue;
                        }

                        match parent.kind() {
                            "type_identifier" | "primitive_type" => {
                                highlights.push(HighlightRange {
                                    start: node.start_byte(),
                                    end: node.end_byte(),
                                    style: HighlightStyle::from_color(
                                        self.current_syntax_theme.type_color.clone(),
                                    ),
                                });
                            }
                            _ => {
                                // Regular identifier - could be variable, function name, etc.
                                highlights.push(HighlightRange {
                                    start: node.start_byte(),
                                    end: node.end_byte(),
                                    style: HighlightStyle::from_color(
                                        self.current_syntax_theme.variable.clone(),
                                    ),
                                });
                            }
                        }
                    }
                }
                "type_identifier" | "primitive_type" => {
                    highlights.push(HighlightRange {
                        start: node.start_byte(),
                        end: node.end_byte(),
                        style: HighlightStyle::from_color(
                            self.current_syntax_theme.type_color.clone(),
                        ),
                    });
                }
                "field_identifier" => {
                    highlights.push(HighlightRange {
                        start: node.start_byte(),
                        end: node.end_byte(),
                        style: HighlightStyle::from_color(
                            self.current_syntax_theme.property.clone(),
                        ),
                    });
                }
                // Keywords - Tree-sitter recognizes these as their literal text
                "use" | "fn" | "let" | "mut" | "if" | "else" | "for" | "while" | "loop"
                | "match" | "return" | "break" | "continue" | "struct" | "enum" | "impl"
                | "trait" | "type" | "const" | "static" | "mod" | "extern" | "pub" | "async"
                | "await" | "unsafe" | "where" | "as" | "in" | "self" | "Self" | "super"
                | "crate" => {
                    highlights.push(HighlightRange {
                        start: node.start_byte(),
                        end: node.end_byte(),
                        style: HighlightStyle::from_color(
                            self.current_syntax_theme.keyword.clone(),
                        ),
                    });
                }
                // Punctuation and operators
                "{" | "}" | "(" | ")" | "[" | "]" | ";" | ":" | "," | "." | "=" | "+" | "-"
                | "*" | "/" | "%" | "&" | "|" | "^" | "!" | "<" | ">" | "?" | "==" | "!="
                | "<=" | ">=" | "&&" | "||" | "++" | "--" | "+=" | "-=" | "*=" | "/=" | "%="
                | "&=" | "|=" | "^=" | "<<" | ">>" | "<<=" | ">>=" | "->" | "=>" | "::" | ".."
                | "..=" => {
                    highlights.push(HighlightRange {
                        start: node.start_byte(),
                        end: node.end_byte(),
                        style: HighlightStyle::from_color(
                            self.current_syntax_theme.operator.clone(),
                        ),
                    });
                }
                _ => {
                    // Only rely on Tree-sitter's node types
                }
            }

            // Add children to stack
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    stack.push(child);
                }
            }
        }

        // Sort highlights by start position
        highlights.sort_by_key(|h| h.start);

        Ok(highlights)
    }

    pub fn highlight_line(
        &mut self,
        line: &str,
        language: &str,
    ) -> Vec<(usize, usize, HighlightType)> {
        // Legacy method for backward compatibility
        if let Ok(highlights) = self.highlight_text(line, language) {
            highlights
                .into_iter()
                .map(|h| {
                    // Try to map highlights to basic types by checking patterns
                    let text_segment = &line[h.start..h.end];
                    let highlight_type = if text_segment.contains("//")
                        || text_segment.contains("/*")
                    {
                        HighlightType::Comment
                    } else if text_segment.starts_with('"') || text_segment.starts_with('\'') {
                        HighlightType::String
                    } else if text_segment.chars().all(|c| c.is_numeric() || c == '.') {
                        HighlightType::Number
                    } else if text_segment == "fn" || text_segment == "let" || text_segment == "if"
                    {
                        HighlightType::Keyword
                    } else if text_segment
                        .chars()
                        .next()
                        .map_or(false, |c| c.is_uppercase())
                    {
                        HighlightType::Type
                    } else {
                        HighlightType::Variable
                    };
                    (h.start, h.end, highlight_type)
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    pub fn reload_config(&mut self) -> Result<()> {
        // Since we no longer use external config, just reinitialize parsers
        self.parsers.clear();
        self.initialize_parsers()?;
        Ok(())
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

        // Check that doc comments are not duplicated
        let doc_comment_highlights: Vec<_> = highlights
            .iter()
            .filter(|h| {
                let text = &test_code[h.start..h.end];
                text.contains("///") || text.contains("//!")
            })
            .collect();

        // Should have exactly 2 highlights: one for /// and one for //!
        assert_eq!(
            doc_comment_highlights.len(),
            2,
            "Should have exactly 2 doc comment highlights (/// and //!)"
        );

        // Check that regular comments work too
        let regular_comment_highlights: Vec<_> = highlights
            .iter()
            .filter(|h| {
                let text = &test_code[h.start..h.end];
                text.starts_with("// Regular") || text.starts_with("/* Block")
            })
            .collect();

        assert_eq!(
            regular_comment_highlights.len(),
            2,
            "Should have exactly 2 regular comment highlights"
        );

        // Ensure no individual slashes are highlighted separately from comments
        let individual_slash_highlights: Vec<_> = highlights
            .iter()
            .filter(|h| {
                let text = &test_code[h.start..h.end];
                text == "/" && h.end - h.start == 1
            })
            .collect();

        assert_eq!(
            individual_slash_highlights.len(),
            0,
            "Should not have individual slash highlights separate from comments"
        );
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
}

#[derive(Debug, Clone, Copy)]
pub enum HighlightType {
    Keyword,
    String,
    Comment,
    Function,
    Variable,
    Type,
    Number,
    Operator,
}

impl From<HighlightType> for HighlightStyle {
    fn from(highlight_type: HighlightType) -> Self {
        // For now, create basic styles. In a full implementation,
        // this would use the theme configuration
        match highlight_type {
            HighlightType::Keyword => HighlightStyle {
                fg_color: Some("#569cd6".to_string()), // Blue
                bg_color: None,
                bold: true,
                italic: false,
                underline: false,
            },
            HighlightType::String => HighlightStyle {
                fg_color: Some("#ce9178".to_string()), // Orange
                bg_color: None,
                bold: false,
                italic: false,
                underline: false,
            },
            HighlightType::Comment => HighlightStyle {
                fg_color: Some("#6a9955".to_string()), // Green
                bg_color: None,
                bold: false,
                italic: true,
                underline: false,
            },
            HighlightType::Function => HighlightStyle {
                fg_color: Some("#dcdcaa".to_string()), // Yellow
                bg_color: None,
                bold: false,
                italic: false,
                underline: false,
            },
            HighlightType::Variable => HighlightStyle {
                fg_color: Some("#9cdcfe".to_string()), // Light blue
                bg_color: None,
                bold: false,
                italic: false,
                underline: false,
            },
            HighlightType::Type => HighlightStyle {
                fg_color: Some("#4ec9b0".to_string()), // Teal
                bg_color: None,
                bold: false,
                italic: false,
                underline: false,
            },
            HighlightType::Number => HighlightStyle {
                fg_color: Some("#b5cea8".to_string()), // Light green
                bg_color: None,
                bold: false,
                italic: false,
                underline: false,
            },
            HighlightType::Operator => HighlightStyle {
                fg_color: Some("#d4d4d4".to_string()), // Light gray
                bg_color: None,
                bold: false,
                italic: false,
                underline: false,
            },
        }
    }
}
