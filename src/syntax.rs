use anyhow::{Result, anyhow};
use crossterm::style::Color;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tree_sitter::{Language, Parser, Query};

// Get the Rust language from the tree-sitter-rust crate
fn get_rust_language() -> Language {
    tree_sitter_rust::LANGUAGE.into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntaxConfig {
    pub general: GeneralConfig,
    pub languages: HashMap<String, LanguageConfig>,
    pub themes: HashMap<String, ThemeConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub enabled: bool,
    pub default_theme: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageConfig {
    pub name: String,
    pub extensions: Vec<String>,
    pub grammar: String,
    pub comment_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    pub keyword: Option<StyleConfig>,
    pub string: Option<StyleConfig>,
    pub comment: Option<StyleConfig>,
    pub number: Option<StyleConfig>,
    pub boolean: Option<StyleConfig>,
    pub operator: Option<StyleConfig>,
    pub punctuation: Option<StyleConfig>,
    pub function: Option<StyleConfig>,
    #[serde(rename = "type")]
    pub type_: Option<StyleConfig>,
    pub variable: Option<StyleConfig>,
    pub constant: Option<StyleConfig>,
    pub property: Option<StyleConfig>,
    pub escape: Option<StyleConfig>,
    pub error: Option<StyleConfig>,
    pub warning: Option<StyleConfig>,
    pub heading: Option<StyleConfig>,
    pub emphasis: Option<StyleConfig>,
    pub strong: Option<StyleConfig>,
    pub code: Option<StyleConfig>,
    pub link: Option<StyleConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleConfig {
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct HighlightRange {
    pub start: usize,
    pub end: usize,
    pub style: StyleConfig,
}

pub struct SyntaxHighlighter {
    config: SyntaxConfig,
    parsers: HashMap<String, Parser>,
    queries: HashMap<String, Query>,
}

impl SyntaxHighlighter {
    pub fn new() -> Result<Self> {
        let config = Self::load_config()?;
        let mut highlighter = SyntaxHighlighter {
            config,
            parsers: HashMap::new(),
            queries: HashMap::new(),
        };

        highlighter.initialize_parsers()?;
        Ok(highlighter)
    }

    fn load_config() -> Result<SyntaxConfig> {
        let config_content = std::fs::read_to_string("syntax.toml")
            .map_err(|_| anyhow!("Could not read syntax.toml"))?;

        let config: SyntaxConfig = toml::from_str(&config_content)
            .map_err(|e| anyhow!("Failed to parse syntax.toml: {}", e))?;

        Ok(config)
    }

    fn initialize_parsers(&mut self) -> Result<()> {
        for (lang_key, lang_config) in &self.config.languages {
            let language = match lang_config.grammar.as_str() {
                "rust" => get_rust_language(),
                _ => continue, // Only Rust supported for now
            };

            let mut parser = Parser::new();
            parser
                .set_language(&language)
                .map_err(|e| anyhow!("Failed to set language for {}: {}", lang_key, e))?;

            self.parsers.insert(lang_key.clone(), parser);

            // Create basic highlighting query for this language
            let query_string = self.create_highlight_query(&lang_config.grammar);

            if let Ok(query) = Query::new(&language, &query_string) {
                self.queries.insert(lang_key.clone(), query);
            }
        }

        Ok(())
    }

    fn create_highlight_query(&self, grammar: &str) -> String {
        match grammar {
            "rust" => {
                // Simplified query - let Tree-sitter handle the structure
                ""
            },
            "javascript" => r#"
                (string) @string
                (template_string) @string
                (comment) @comment
                (number) @number
                (true) @boolean
                (false) @boolean
                ["function" "const" "let" "var" "if" "else" "for" "while" "return" "class" "extends" "import" "export" "from" "as" "default"] @keyword
                (function_declaration name: (identifier) @function)
                (call_expression function: (identifier) @function)
                (property_identifier) @property
            "#,
            "python" => r#"
                (string) @string
                (comment) @comment
                (integer) @number
                (float) @number
                (true) @boolean
                (false) @boolean
                (none) @constant
                ["def" "class" "if" "elif" "else" "for" "while" "return" "import" "from" "as" "try" "except" "finally" "with" "lambda" "and" "or" "not" "in" "is"] @keyword
                (function_definition name: (identifier) @function)
                (call function: (identifier) @function)
                (attribute attribute: (identifier) @property)
            "#,
            "markdown" => r#"
                (atx_heading) @heading
                (code_span) @code
                (fenced_code_block) @code
                (emphasis) @emphasis
                (strong_emphasis) @strong
                (link) @link
            "#,
            "json" => r#"
                (string) @string
                (number) @number
                (true) @boolean
                (false) @boolean
                (null) @constant
            "#,
            "toml" => r#"
                (string) @string
                (comment) @comment
                (integer) @number
                (float) @number
                (boolean) @boolean
                (bare_key) @property
                (quoted_key) @property
            "#,
            _ => "",
        }.to_string()
    }

    pub fn detect_language_from_extension(&self, file_path: &str) -> Option<String> {
        let extension = Path::new(file_path).extension()?.to_str()?;

        for (lang_key, lang_config) in &self.config.languages {
            if lang_config.extensions.contains(&extension.to_string()) {
                return Some(lang_key.clone());
            }
        }

        None
    }

    pub fn update_theme(&mut self, theme_name: &str) -> Result<()> {
        // Reload the config to get updated themes
        self.config = Self::load_config()?;
        
        // Validate that the theme exists
        if !self.config.themes.contains_key(theme_name) {
            return Err(anyhow!("Theme '{}' not found", theme_name));
        }
        
        // Update the default theme in the config
        self.config.general.default_theme = theme_name.to_string();
        
        Ok(())
    }

    pub fn highlight_text(&mut self, text: &str, language: &str) -> Result<Vec<HighlightRange>> {
        if !self.config.general.enabled {
            return Ok(Vec::new());
        }

        let parser = self
            .parsers
            .get_mut(language)
            .ok_or_else(|| anyhow!("No parser for language: {}", language))?;

        let tree = parser
            .parse(text, None)
            .ok_or_else(|| anyhow!("Failed to parse text"))?;

        let mut highlights = Vec::new();

        let theme = self
            .config
            .themes
            .get(&self.config.general.default_theme)
            .ok_or_else(|| anyhow!("Default theme not found"))?;

        // Use a simplified approach for Tree-sitter 0.25.8
        // Try a different approach - query all nodes manually
        let mut stack = vec![tree.root_node()];

        while let Some(node) = stack.pop() {
            let node_kind = node.kind();

            // Check for Tree-sitter node types first (more accurate)
            match node_kind {
                "string_literal" | "raw_string_literal" | "char_literal" => {
                    if let Some(style) = self.get_style_for_capture(theme, "string") {
                        highlights.push(HighlightRange {
                            start: node.start_byte(),
                            end: node.end_byte(),
                            style: style.clone(),
                        });
                    }
                }
                "line_comment" | "block_comment" => {
                    if let Some(style) = self.get_style_for_capture(theme, "comment") {
                        highlights.push(HighlightRange {
                            start: node.start_byte(),
                            end: node.end_byte(),
                            style: style.clone(),
                        });
                    }
                }
                "integer_literal" | "float_literal" => {
                    if let Some(style) = self.get_style_for_capture(theme, "number") {
                        highlights.push(HighlightRange {
                            start: node.start_byte(),
                            end: node.end_byte(),
                            style: style.clone(),
                        });
                    }
                }
                "boolean_literal" => {
                    if let Some(style) = self.get_style_for_capture(theme, "boolean") {
                        highlights.push(HighlightRange {
                            start: node.start_byte(),
                            end: node.end_byte(),
                            style: style.clone(),
                        });
                    }
                }
                "macro_invocation" => {
                    // For macro invocations, highlight only the macro name (first child), not the entire call
                    if let Some(macro_name_node) = node.child(0) {
                        if let Some(style) = self.get_style_for_capture(theme, "function") {
                            highlights.push(HighlightRange {
                                start: macro_name_node.start_byte(),
                                end: macro_name_node.end_byte(),
                                style: style.clone(),
                            });
                        }
                    }
                }
                "identifier" => {
                    // Check if this identifier is in a type position
                    if let Some(parent) = node.parent() {
                        match parent.kind() {
                            "type_identifier" | "primitive_type" => {
                                if let Some(style) = self.get_style_for_capture(theme, "type") {
                                    highlights.push(HighlightRange {
                                        start: node.start_byte(),
                                        end: node.end_byte(),
                                        style: style.clone(),
                                    });
                                }
                            }
                            _ => {
                                // Regular identifier - could be variable, function name, etc.
                                if let Some(style) = self.get_style_for_capture(theme, "variable") {
                                    highlights.push(HighlightRange {
                                        start: node.start_byte(),
                                        end: node.end_byte(),
                                        style: style.clone(),
                                    });
                                }
                            }
                        }
                    }
                }
                "type_identifier" | "primitive_type" => {
                    if let Some(style) = self.get_style_for_capture(theme, "type") {
                        highlights.push(HighlightRange {
                            start: node.start_byte(),
                            end: node.end_byte(),
                            style: style.clone(),
                        });
                    }
                }
                "field_identifier" => {
                    if let Some(style) = self.get_style_for_capture(theme, "property") {
                        highlights.push(HighlightRange {
                            start: node.start_byte(),
                            end: node.end_byte(),
                            style: style.clone(),
                        });
                    }
                }
                // Keywords - Tree-sitter recognizes these as their literal text
                "use" | "fn" | "let" | "mut" | "if" | "else" | "for" | "while" | "loop"
                | "match" | "return" | "break" | "continue" | "struct" | "enum" | "impl"
                | "trait" | "type" | "const" | "static" | "mod" | "extern" | "pub" | "async"
                | "await" | "unsafe" | "where" | "as" | "in" | "self" | "Self" | "super"
                | "crate" => {
                    if let Some(style) = self.get_style_for_capture(theme, "keyword") {
                        highlights.push(HighlightRange {
                            start: node.start_byte(),
                            end: node.end_byte(),
                            style: style.clone(),
                        });
                    }
                }
                _ => {
                    // Only rely on Tree-sitter's node types - no fallback text matching
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
                    let highlight_type = match h.style.fg.as_deref() {
                        Some("#569cd6") => HighlightType::Keyword,
                        Some("#ce9178") => HighlightType::String,
                        Some("#6a9955") => HighlightType::Comment,
                        Some("#dcdcaa") => HighlightType::Function,
                        Some("#4ec9b0") => HighlightType::Type,
                        Some("#b5cea8") => HighlightType::Number,
                        _ => HighlightType::Variable,
                    };
                    (h.start, h.end, highlight_type)
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    fn get_style_for_capture<'a>(
        &self,
        theme: &'a ThemeConfig,
        capture_name: &str,
    ) -> Option<&'a StyleConfig> {
        match capture_name {
            "keyword" => theme.keyword.as_ref(),
            "string" => theme.string.as_ref(),
            "comment" => theme.comment.as_ref(),
            "number" => theme.number.as_ref(),
            "boolean" => theme.boolean.as_ref(),
            "operator" => theme.operator.as_ref(),
            "punctuation" => theme.punctuation.as_ref(),
            "function" => theme.function.as_ref(),
            "type" => theme.type_.as_ref(),
            "variable" => theme.variable.as_ref(),
            "constant" => theme.constant.as_ref(),
            "property" => theme.property.as_ref(),
            "escape" => theme.escape.as_ref(),
            "error" => theme.error.as_ref(),
            "warning" => theme.warning.as_ref(),
            "heading" => theme.heading.as_ref(),
            "emphasis" => theme.emphasis.as_ref(),
            "strong" => theme.strong.as_ref(),
            "code" => theme.code.as_ref(),
            "link" => theme.link.as_ref(),
            _ => None,
        }
    }

    pub fn reload_config(&mut self) -> Result<()> {
        self.config = Self::load_config()?;
        self.parsers.clear();
        self.queries.clear();
        self.initialize_parsers()?;
        Ok(())
    }
}

impl StyleConfig {
    pub fn to_color(&self) -> Option<Color> {
        self.fg.as_ref().and_then(|color_str| {
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
                    "#569cd6" => Some(Color::Blue), // keyword color -> blue
                    "#ce9178" => Some(Color::Yellow), // string color -> yellow
                    "#6a9955" => Some(Color::Green), // comment color -> green
                    "#b5cea8" => Some(Color::Cyan), // number color -> cyan
                    "#dcdcaa" => Some(Color::Yellow), // function color -> yellow
                    "#4ec9b0" => Some(Color::Cyan), // type color -> cyan
                    _ => None,
                }
            }
        })
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
