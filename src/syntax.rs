use anyhow::{Result, anyhow};
use crossterm::style::Color;
use std::collections::HashMap;
use std::path::Path;
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
            Color::Black => "#000000".to_string(),
            Color::Red => "#ff0000".to_string(),
            Color::Green => "#00ff00".to_string(),
            Color::Yellow => "#ffff00".to_string(),
            Color::Blue => "#0000ff".to_string(),
            Color::Magenta => "#ff00ff".to_string(),
            Color::Cyan => "#00ffff".to_string(),
            Color::White => "#ffffff".to_string(),
            Color::DarkGrey => "#808080".to_string(),
            Color::Grey => "#c0c0c0".to_string(),
            _ => "#ffffff".to_string(), // Default fallback
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
                // Fallback to basic colors for better terminal compatibility
                match color_str.as_str() {
                    "blue" => Some(Color::Blue),
                    "red" => Some(Color::Red),
                    "green" => Some(Color::Green),
                    "yellow" => Some(Color::Yellow),
                    "magenta" => Some(Color::Magenta),
                    "cyan" => Some(Color::Cyan),
                    "white" => Some(Color::White),
                    "black" => Some(Color::Black),
                    _ => None,
                }
            }
        })
    }
}

pub struct SyntaxHighlighter {
    parsers: HashMap<String, Parser>,
    theme_config: ThemeConfig,
    current_syntax_theme: SyntaxTheme,
}

impl SyntaxHighlighter {
    pub fn new() -> Result<Self> {
        // Load the theme system
        let theme_config = ThemeConfig::load();
        let current_theme = theme_config.get_current_theme();

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

        Ok(())
    }

    pub fn highlight_text(&mut self, text: &str, language: &str) -> Result<Vec<HighlightRange>> {
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
                                self.current_syntax_theme.function.clone(),
                            ),
                        });
                    }
                }
                "identifier" => {
                    // Check if this identifier is in a type position
                    if let Some(parent) = node.parent() {
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
