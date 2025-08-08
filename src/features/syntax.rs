use anyhow::{Result, anyhow};
use crossterm::style::Color;
use log::{debug, info, trace, warn};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use tree_sitter::{Language, Parser};

use crate::config::theme::{SyntaxTheme, ThemeConfig};

/// Semantic highlighting categories for generic syntax highlighting
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum SemanticCategory {
    // Keywords and control flow
    Keyword,
    Conditional,
    Repeat,
    Exception,
    StorageClass,

    // Identifiers and names
    Identifier,
    Function,
    Method,
    Parameter,
    Variable,
    Property,
    Field,

    // Types and declarations
    Type,
    Class,
    Struct,
    Interface,
    Enum,
    Constant,

    // Literals
    String,
    Number,
    Boolean,
    Character,

    // Comments and documentation
    Comment,
    Documentation,

    // Operators and punctuation
    Operator,
    Punctuation,
    Delimiter,

    // Special constructs
    Preprocessor,
    Macro,
    Attribute,
    Label,

    // Generic fallback
    Text,
}

impl SemanticCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            SemanticCategory::Keyword => "keyword",
            SemanticCategory::Conditional => "conditional",
            SemanticCategory::Repeat => "repeat",
            SemanticCategory::Exception => "exception",
            SemanticCategory::StorageClass => "storage_class",
            SemanticCategory::Identifier => "identifier",
            SemanticCategory::Function => "function",
            SemanticCategory::Method => "method",
            SemanticCategory::Parameter => "parameter",
            SemanticCategory::Variable => "variable",
            SemanticCategory::Property => "property",
            SemanticCategory::Field => "field",
            SemanticCategory::Type => "type",
            SemanticCategory::Class => "class",
            SemanticCategory::Struct => "struct",
            SemanticCategory::Interface => "interface",
            SemanticCategory::Enum => "enum",
            SemanticCategory::Constant => "constant",
            SemanticCategory::String => "string",
            SemanticCategory::Number => "number",
            SemanticCategory::Boolean => "boolean",
            SemanticCategory::Character => "character",
            SemanticCategory::Comment => "comment",
            SemanticCategory::Documentation => "documentation",
            SemanticCategory::Operator => "operator",
            SemanticCategory::Punctuation => "punctuation",
            SemanticCategory::Delimiter => "delimiter",
            SemanticCategory::Preprocessor => "preprocessor",
            SemanticCategory::Macro => "macro",
            SemanticCategory::Attribute => "attribute",
            SemanticCategory::Label => "label",
            SemanticCategory::Text => "text",
        }
    }
}

/// Language support configuration
#[derive(Debug, Clone)]
pub struct LanguageSupport {
    pub name: String,
    pub extensions: Vec<String>,
    pub tree_sitter_name: String,
    pub node_mappings: HashMap<String, SemanticCategory>,
}

impl LanguageSupport {
    /// Create default Rust language support  
    pub fn rust() -> Self {
        let mut node_mappings = HashMap::new();

        // Keywords
        node_mappings.insert("fn".to_string(), SemanticCategory::Keyword);
        node_mappings.insert("let".to_string(), SemanticCategory::Keyword);
        node_mappings.insert("mut".to_string(), SemanticCategory::Keyword);
        node_mappings.insert("const".to_string(), SemanticCategory::Keyword);
        node_mappings.insert("static".to_string(), SemanticCategory::Keyword);
        node_mappings.insert("struct".to_string(), SemanticCategory::Keyword);
        node_mappings.insert("enum".to_string(), SemanticCategory::Keyword);
        node_mappings.insert("impl".to_string(), SemanticCategory::Keyword);
        node_mappings.insert("trait".to_string(), SemanticCategory::Keyword);
        node_mappings.insert("type".to_string(), SemanticCategory::Keyword);
        node_mappings.insert("mod".to_string(), SemanticCategory::Keyword);
        node_mappings.insert("use".to_string(), SemanticCategory::Keyword);
        node_mappings.insert("pub".to_string(), SemanticCategory::Keyword);
        node_mappings.insert("extern".to_string(), SemanticCategory::Keyword);
        node_mappings.insert("unsafe".to_string(), SemanticCategory::Keyword);
        node_mappings.insert("async".to_string(), SemanticCategory::Keyword);
        node_mappings.insert("await".to_string(), SemanticCategory::Keyword);

        // Control flow
        node_mappings.insert("if".to_string(), SemanticCategory::Conditional);
        node_mappings.insert("else".to_string(), SemanticCategory::Conditional);
        node_mappings.insert("match".to_string(), SemanticCategory::Conditional);
        node_mappings.insert("for".to_string(), SemanticCategory::Repeat);
        node_mappings.insert("while".to_string(), SemanticCategory::Repeat);
        node_mappings.insert("loop".to_string(), SemanticCategory::Repeat);
        node_mappings.insert("return".to_string(), SemanticCategory::Exception);
        node_mappings.insert("break".to_string(), SemanticCategory::Exception);
        node_mappings.insert("continue".to_string(), SemanticCategory::Exception);

        // Types and identifiers
        node_mappings.insert("type_identifier".to_string(), SemanticCategory::Type);
        node_mappings.insert("primitive_type".to_string(), SemanticCategory::Type);
        node_mappings.insert("field_identifier".to_string(), SemanticCategory::Field);
        node_mappings.insert(
            "function_identifier".to_string(),
            SemanticCategory::Function,
        );
        node_mappings.insert("identifier".to_string(), SemanticCategory::Identifier);
        node_mappings.insert(
            "scoped_identifier".to_string(),
            SemanticCategory::Identifier,
        );

        // Literals
        node_mappings.insert("string_literal".to_string(), SemanticCategory::String);
        node_mappings.insert("raw_string_literal".to_string(), SemanticCategory::String);
        node_mappings.insert("char_literal".to_string(), SemanticCategory::Character);
        node_mappings.insert("integer_literal".to_string(), SemanticCategory::Number);
        node_mappings.insert("float_literal".to_string(), SemanticCategory::Number);
        node_mappings.insert("boolean_literal".to_string(), SemanticCategory::Boolean);

        // Comments
        node_mappings.insert("line_comment".to_string(), SemanticCategory::Comment);
        node_mappings.insert("block_comment".to_string(), SemanticCategory::Comment);
        node_mappings.insert("doc_comment".to_string(), SemanticCategory::Documentation);
        node_mappings.insert(
            "outer_doc_comment_marker".to_string(),
            SemanticCategory::Documentation,
        );
        node_mappings.insert(
            "inner_doc_comment_marker".to_string(),
            SemanticCategory::Documentation,
        );

        // Punctuation (simplified - using fewer specific mappings)
        node_mappings.insert(";".to_string(), SemanticCategory::Punctuation);
        node_mappings.insert(",".to_string(), SemanticCategory::Punctuation);
        node_mappings.insert(".".to_string(), SemanticCategory::Punctuation);
        node_mappings.insert(":".to_string(), SemanticCategory::Punctuation);
        node_mappings.insert("::".to_string(), SemanticCategory::Punctuation);
        node_mappings.insert("=".to_string(), SemanticCategory::Operator);
        node_mappings.insert("+".to_string(), SemanticCategory::Operator);
        node_mappings.insert("-".to_string(), SemanticCategory::Operator);
        node_mappings.insert("*".to_string(), SemanticCategory::Operator);
        node_mappings.insert("/".to_string(), SemanticCategory::Operator);
        node_mappings.insert("!".to_string(), SemanticCategory::Operator);
        node_mappings.insert("&".to_string(), SemanticCategory::Operator);
        node_mappings.insert("|".to_string(), SemanticCategory::Operator);
        node_mappings.insert("?".to_string(), SemanticCategory::Operator);

        // Delimiters
        node_mappings.insert("(".to_string(), SemanticCategory::Delimiter);
        node_mappings.insert(")".to_string(), SemanticCategory::Delimiter);
        node_mappings.insert("{".to_string(), SemanticCategory::Delimiter);
        node_mappings.insert("}".to_string(), SemanticCategory::Delimiter);
        node_mappings.insert("[".to_string(), SemanticCategory::Delimiter);
        node_mappings.insert("]".to_string(), SemanticCategory::Delimiter);

        // Macros and attributes
        node_mappings.insert("macro_invocation".to_string(), SemanticCategory::Macro);
        node_mappings.insert("attribute".to_string(), SemanticCategory::Attribute);

        LanguageSupport {
            name: "rust".to_string(),
            extensions: vec!["rs".to_string()],
            tree_sitter_name: "rust".to_string(),
            node_mappings,
        }
    }

    /// Get all supported languages (for now just Rust, but extensible)
    pub fn get_all_languages() -> Vec<LanguageSupport> {
        vec![LanguageSupport::rust()]
    }
}

/// Get a tree-sitter language by name - returns None if not supported
fn get_tree_sitter_language(language_name: &str) -> Option<Language> {
    match language_name {
        "rust" => Some(tree_sitter_rust::LANGUAGE.into()),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HighlightRange {
    pub start: usize,
    pub end: usize,
    pub style: HighlightStyle,
}

#[derive(Debug, Clone, PartialEq)]
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
        let theme_config = crate::config::theme::ThemeConfig::load();
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
    language_support: HashMap<String, LanguageSupport>,
    theme_config: ThemeConfig,
    current_syntax_theme: SyntaxTheme,
}

impl SyntaxHighlighter {
    pub fn new() -> Result<Self> {
        info!("Initializing generic syntax highlighter");

        // Load the theme system
        let theme_config = ThemeConfig::load();
        let current_theme = theme_config.get_current_theme();
        debug!("Loaded syntax theme: '{}'", current_theme.name);

        // Initialize language support
        let mut language_support = HashMap::new();
        for lang in LanguageSupport::get_all_languages() {
            language_support.insert(lang.name.clone(), lang);
        }
        debug!("Loaded {} language definitions", language_support.len());

        let mut highlighter = SyntaxHighlighter {
            parsers: HashMap::new(),
            language_support,
            theme_config,
            current_syntax_theme: current_theme.syntax,
        };

        highlighter.initialize_parsers()?;
        Ok(highlighter)
    }

    fn initialize_parsers(&mut self) -> Result<()> {
        for (lang_name, lang_support) in &self.language_support {
            if let Some(language) = get_tree_sitter_language(&lang_support.tree_sitter_name) {
                let mut parser = Parser::new();
                parser
                    .set_language(&language)
                    .map_err(|e| anyhow!("Failed to set language for {}: {}", lang_name, e))?;

                self.parsers.insert(lang_name.clone(), parser);
                debug!("Initialized parser for language: {}", lang_name);
            } else {
                warn!("Tree-sitter language not available for: {}", lang_name);
            }
        }

        debug!("Initialized {} parsers", self.parsers.len());
        Ok(())
    }

    pub fn detect_language_from_extension(&self, file_path: &str) -> Option<String> {
        use std::path::Path;

        let extension = Path::new(file_path).extension()?.to_str()?;

        // Check our supported languages for this extension
        for (lang_name, lang_support) in &self.language_support {
            if lang_support.extensions.contains(&extension.to_string()) {
                return Some(lang_name.clone());
            }
        }

        // Fall back to editor config
        let config = crate::config::EditorConfig::load();
        config.languages.detect_language_from_extension(file_path)
    }

    /// Detect language from content patterns for unnamed files
    pub fn detect_language_from_content(&self, content: &str) -> Option<String> {
        // For now, simple heuristics - can be enhanced later
        if content.contains("fn main()") || content.contains("use std::") {
            return Some("rust".to_string());
        }

        // Fall back to editor config
        let config = crate::config::EditorConfig::load();
        config.languages.detect_language_from_content(content)
    }

    /// Detect language using both file path and content fallback
    pub fn detect_language(&self, file_path: Option<&str>, content: &str) -> String {
        if let Some(path) = file_path {
            if let Some(language) = self.detect_language_from_extension(path) {
                return language;
            }
        }

        // Fall back to content-based detection
        self.detect_language_from_content(content)
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

        let language_support = self
            .language_support
            .get(language)
            .ok_or_else(|| anyhow!("No language support for: {}", language))?;

        let tree = parser
            .parse(text, None)
            .ok_or_else(|| anyhow!("Failed to parse text"))?;

        let mut highlights = Vec::new();

        // Simple approach - just add all matching nodes without complex conflict resolution
        let mut stack = vec![tree.root_node()];

        while let Some(node) = stack.pop() {
            let node_kind = node.kind();

            // Debug: Log node kinds for development
            if log::log_enabled!(log::Level::Trace) {
                if let Ok(node_text) = node.utf8_text(text.as_bytes()) {
                    if node_text.len() < 20 && !node_text.contains('\n') {
                        trace!("Node type: '{}' -> text: '{}'", node_kind, node_text);
                    }
                }
            }

            // Check if this node type maps to a semantic category
            if let Some(semantic_category) = language_support.node_mappings.get(node_kind) {
                // Get the color for this semantic category from the theme
                if let Some(color) = self.get_color_for_category(semantic_category) {
                    // Add highlight directly
                    highlights.push(HighlightRange {
                        start: node.start_byte(),
                        end: node.end_byte(),
                        style: HighlightStyle::from_color(color),
                    });
                }
            }

            // Add children to stack for processing
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    stack.push(child);
                }
            }
        }

        // Sort highlights by start position
        highlights.sort_by_key(|h| h.start);

        let original_count = highlights.len();

        // Efficient overlap removal using a single pass
        if highlights.len() <= 1 {
            debug!(
                "Generated {} highlights for {} content",
                highlights.len(),
                language
            );
            return Ok(highlights);
        }

        // Sort by start position, then by end position (smaller ranges first for same start)
        highlights.sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

        // Remove overlapping ranges in a single pass - keep only non-overlapping ranges
        let mut filtered_highlights = Vec::with_capacity(highlights.len());
        let mut last_end = 0;

        for highlight in highlights {
            // Only keep highlights that don't overlap with previous ones
            if highlight.start >= last_end {
                last_end = highlight.end;
                filtered_highlights.push(highlight);
            }
            // Skip overlapping ranges (they get dropped automatically)
        }

        debug!(
            "Generated {} highlights for {} content (filtered from {} total)",
            filtered_highlights.len(),
            language,
            original_count
        );
        Ok(filtered_highlights)
    }

    /// Get color for a semantic category from the current theme
    fn get_color_for_category(&self, category: &SemanticCategory) -> Option<Color> {
        // Look up the color for this semantic category in the theme
        // The theme now uses semantic category names instead of specific node types
        self.current_syntax_theme
            .tree_sitter_mappings
            .get(category.as_str())
            .cloned()
    }

    pub fn reload_config(&mut self) -> Result<()> {
        // Reinitialize language support and parsers
        self.language_support.clear();
        for lang in LanguageSupport::get_all_languages() {
            self.language_support.insert(lang.name.clone(), lang);
        }

        self.parsers.clear();
        self.initialize_parsers()?;
        Ok(())
    }

    /// Get the current theme mappings for testing
    pub fn get_theme_mappings(&self) -> &HashMap<String, Color> {
        &self.current_syntax_theme.tree_sitter_mappings
    }

    /// Get the supported languages for testing
    pub fn get_supported_languages(&self) -> Vec<&str> {
        self.language_support.keys().map(|s| s.as_str()).collect()
    }
}

/// Priority levels for syntax highlighting requests (simplified version)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low = 0,
    Medium = 1,
    High = 2,
    Critical = 3,
}

/// Simplified async syntax highlighter for compatibility
pub struct AsyncSyntaxHighlighter {
    sync_highlighter: SyntaxHighlighter,
}

impl AsyncSyntaxHighlighter {
    pub fn new() -> Result<Self> {
        Ok(AsyncSyntaxHighlighter {
            sync_highlighter: SyntaxHighlighter::new()?,
        })
    }

    pub fn get_cached_highlights(
        &self,
        _buffer_id: usize,
        _line_index: usize,
        content: &str,
        language: &str,
    ) -> Option<Vec<HighlightRange>> {
        // Only highlight single lines, not entire files
        if content.lines().count() > 1 {
            // Don't highlight multi-line content synchronously - too expensive
            return None;
        }

        // For single lines, do quick highlighting
        let mut highlighter = self.sync_highlighter.clone();
        highlighter.highlight_text(content, language).ok()
    }

    pub fn force_immediate_highlights_with_context(
        &self,
        _buffer_id: usize,
        _line_index: usize,
        content: &str,
        _text: &str,
        language: &str,
    ) -> Option<Vec<HighlightRange>> {
        // Don't do expensive full-file highlighting synchronously
        // Instead, just highlight individual lines for immediate display
        if content.lines().count() > 1 {
            warn!(
                "Refusing to do synchronous highlighting of {} lines - too expensive!",
                content.lines().count()
            );
            return None;
        }

        let mut highlighter = self.sync_highlighter.clone();
        highlighter.highlight_text(content, language).ok()
    }

    pub fn request_highlighting(
        &self,
        _buffer_id: usize,
        _line_index: usize,
        content: String,
        language: String,
        _priority: Priority,
    ) {
        // For simplified version, we don't queue requests
        let mut highlighter = self.sync_highlighter.clone();
        let _ = highlighter.highlight_text(&content, &language);
    }

    pub fn cache_stats(&self) -> (usize, usize) {
        (0, 0) // Return simple cache stats format
    }

    pub fn update_theme(&self, theme_name: &str) -> Result<()> {
        // For simplified version, we don't actually update the theme
        // Just return ok to maintain compatibility
        log::info!(
            "Theme update request for '{}' (simplified version)",
            theme_name
        );
        Ok(())
    }

    pub fn invalidate_buffer_cache(&self, _buffer_id: usize) {
        // No-op for simplified version
    }
}

impl Clone for SyntaxHighlighter {
    fn clone(&self) -> Self {
        // Create a new instance rather than cloning parsers (which aren't Clone)
        Self::new().unwrap_or_else(|_| SyntaxHighlighter {
            parsers: HashMap::new(),
            language_support: self.language_support.clone(),
            theme_config: self.theme_config.clone(),
            current_syntax_theme: self.current_syntax_theme.clone(),
        })
    }
}
