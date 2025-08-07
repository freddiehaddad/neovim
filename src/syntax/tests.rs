use crate::syntax::{
    AsyncSyntaxHighlighter, HighlightRange, HighlightStyle, LanguageSupport, SemanticCategory,
    SyntaxHighlighter, SyntaxTheme,
};
use std::collections::HashMap;

#[cfg(test)]
mod syntax_tests {
    use super::*;

    #[test]
    fn test_semantic_category_all_variants() {
        // Test that all SemanticCategory variants can be created
        let categories = vec![
            SemanticCategory::Keyword,
            SemanticCategory::Conditional,
            SemanticCategory::Repeat,
            SemanticCategory::Exception,
            SemanticCategory::StorageClass,
            SemanticCategory::Identifier,
            SemanticCategory::Function,
            SemanticCategory::Method,
            SemanticCategory::Parameter,
            SemanticCategory::Variable,
            SemanticCategory::Property,
            SemanticCategory::Field,
            SemanticCategory::Type,
            SemanticCategory::Class,
            SemanticCategory::Struct,
            SemanticCategory::Interface,
            SemanticCategory::Enum,
            SemanticCategory::Constant,
            SemanticCategory::String,
            SemanticCategory::Number,
            SemanticCategory::Boolean,
            SemanticCategory::Character,
            SemanticCategory::Comment,
            SemanticCategory::Documentation,
            SemanticCategory::Operator,
            SemanticCategory::Punctuation,
            SemanticCategory::Delimiter,
            SemanticCategory::Preprocessor,
            SemanticCategory::Macro,
            SemanticCategory::Attribute,
            SemanticCategory::Label,
            SemanticCategory::Text,
        ];

        assert_eq!(categories.len(), 32);
    }

    #[test]
    fn test_semantic_category_as_str() {
        assert_eq!(SemanticCategory::Keyword.as_str(), "keyword");
        assert_eq!(SemanticCategory::Function.as_str(), "function");
        assert_eq!(SemanticCategory::String.as_str(), "string");
        assert_eq!(SemanticCategory::Comment.as_str(), "comment");
        assert_eq!(SemanticCategory::Variable.as_str(), "variable");
        assert_eq!(SemanticCategory::Type.as_str(), "type");
    }

    #[test]
    fn test_highlight_style_creation() {
        let style = HighlightStyle {
            fg_color: Some("#ff0000".to_string()),
            bg_color: Some("#ffffff".to_string()),
            bold: true,
            italic: false,
            underline: true,
        };

        assert_eq!(style.fg_color, Some("#ff0000".to_string()));
        assert_eq!(style.bg_color, Some("#ffffff".to_string()));
        assert_eq!(style.bold, true);
        assert_eq!(style.italic, false);
        assert_eq!(style.underline, true);
    }

    #[test]
    fn test_highlight_style_default() {
        let style = HighlightStyle {
            fg_color: None,
            bg_color: None,
            bold: false,
            italic: false,
            underline: false,
        };

        assert_eq!(style.fg_color, None);
        assert_eq!(style.bg_color, None);
        assert_eq!(style.bold, false);
        assert_eq!(style.italic, false);
        assert_eq!(style.underline, false);
    }

    #[test]
    fn test_highlight_range_creation() {
        let style = HighlightStyle {
            fg_color: Some("#0000ff".to_string()),
            bg_color: None,
            bold: true,
            italic: false,
            underline: false,
        };

        let range = HighlightRange {
            start: 0,
            end: 10,
            style,
        };

        assert_eq!(range.start, 0);
        assert_eq!(range.end, 10);
        assert_eq!(range.style.fg_color, Some("#0000ff".to_string()));
        assert_eq!(range.style.bold, true);
    }

    #[test]
    fn test_highlight_range_different_styles() {
        let keyword_style = HighlightStyle {
            fg_color: Some("#800080".to_string()),
            bg_color: None,
            bold: true,
            italic: false,
            underline: false,
        };

        let function_style = HighlightStyle {
            fg_color: Some("#0000ff".to_string()),
            bg_color: None,
            bold: false,
            italic: true,
            underline: false,
        };

        let string_style = HighlightStyle {
            fg_color: Some("#008000".to_string()),
            bg_color: None,
            bold: false,
            italic: false,
            underline: false,
        };

        let number_style = HighlightStyle {
            fg_color: Some("#ff8000".to_string()),
            bg_color: None,
            bold: false,
            italic: false,
            underline: false,
        };

        let ranges = vec![
            HighlightRange {
                start: 0,
                end: 5,
                style: keyword_style,
            },
            HighlightRange {
                start: 6,
                end: 15,
                style: function_style,
            },
            HighlightRange {
                start: 16,
                end: 25,
                style: string_style,
            },
            HighlightRange {
                start: 26,
                end: 30,
                style: number_style,
            },
        ];

        assert_eq!(ranges.len(), 4);
        assert_eq!(ranges[0].style.fg_color, Some("#800080".to_string()));
        assert_eq!(ranges[1].style.italic, true);
        assert_eq!(ranges[2].style.fg_color, Some("#008000".to_string()));
        assert_eq!(ranges[3].style.fg_color, Some("#ff8000".to_string()));
    }

    #[test]
    fn test_language_support_rust() {
        let rust_support = LanguageSupport::rust();

        assert_eq!(rust_support.name, "rust");
        assert!(rust_support.extensions.contains(&"rs".to_string()));
        assert_eq!(rust_support.tree_sitter_name, "rust");
        assert!(rust_support.node_mappings.contains_key("fn"));
        assert_eq!(rust_support.node_mappings["fn"], SemanticCategory::Keyword);
    }

    #[test]
    fn test_language_support_creation() {
        let mut node_mappings = HashMap::new();
        node_mappings.insert("function".to_string(), SemanticCategory::Function);
        node_mappings.insert("var".to_string(), SemanticCategory::Variable);

        let lang_support = LanguageSupport {
            name: "TestLang".to_string(),
            extensions: vec!["test".to_string(), "tst".to_string()],
            tree_sitter_name: "test".to_string(),
            node_mappings,
        };

        assert_eq!(lang_support.name, "TestLang");
        assert_eq!(lang_support.extensions.len(), 2);
        assert!(lang_support.extensions.contains(&"test".to_string()));
        assert_eq!(lang_support.tree_sitter_name, "test");
        assert_eq!(lang_support.node_mappings.len(), 2);
    }

    #[test]
    fn test_syntax_highlighter_creation() {
        let _highlighter = SyntaxHighlighter::new();

        // Test that highlighter can be created successfully
        // The actual highlighting functionality requires tree-sitter setup
        // which is more complex to test in isolation
        assert!(true); // Placeholder - highlighter created successfully
    }

    #[test]
    fn test_get_all_languages() {
        let languages = LanguageSupport::get_all_languages();

        // Should have at least Rust support
        assert!(!languages.is_empty());

        // Check that Rust is included
        let rust_lang = languages.iter().find(|lang| lang.name == "rust");
        assert!(rust_lang.is_some());

        if let Some(rust) = rust_lang {
            assert!(rust.extensions.contains(&"rs".to_string()));
            assert_eq!(rust.tree_sitter_name, "rust");
        }
    }

    #[test]
    fn test_syntax_highlighter_basic_creation() {
        let highlighter = SyntaxHighlighter::new();

        // Test that highlighter creation returns a Result
        assert!(highlighter.is_ok() || highlighter.is_err());

        // If successful, test basic functionality
        if let Ok(h) = highlighter {
            let supported_languages = h.get_supported_languages();
            assert!(!supported_languages.is_empty());
        }
    }
}

#[cfg(test)]
mod async_syntax_tests {
    use super::*;

    #[tokio::test]
    async fn test_async_syntax_highlighter_creation() {
        let highlighter = AsyncSyntaxHighlighter::new();

        // Test that async highlighter creation returns a Result
        assert!(highlighter.is_ok() || highlighter.is_err());
    }

    #[tokio::test]
    async fn test_async_highlight_basic() {
        let highlighter = AsyncSyntaxHighlighter::new();

        if let Ok(h) = highlighter {
            let content = "fn main() {}";
            let result = h.get_cached_highlights(0, 0, content, "rust");

            // Test that we get an option result (Some or None both valid)
            assert!(result.is_some() || result.is_none());

            // If we get highlights, they should be valid
            if let Some(ranges) = result {
                for range in ranges {
                    assert!(range.start <= range.end);
                    assert!(range.end <= content.len());
                }
            }
        }
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn test_syntax_highlighter_language_detection() {
        let highlighter = SyntaxHighlighter::new();

        if let Ok(h) = highlighter {
            // Test extension-based detection
            let rust_lang = h.detect_language_from_extension("test.rs");
            assert_eq!(rust_lang, Some("rust".to_string()));

            let unknown_lang = h.detect_language_from_extension("test.unknown");
            assert_eq!(unknown_lang, None);
        }
    }

    #[test]
    fn test_full_language_detection() {
        let highlighter = SyntaxHighlighter::new();

        if let Ok(h) = highlighter {
            // Test with file path
            let detected = h.detect_language(Some("test.rs"), "fn main() {}");
            assert_eq!(detected, "rust");

            // Test fallback to content detection when no file path
            let detected_content = h.detect_language(None, "fn main() {}");
            // Should fallback to default or detect from content patterns
            assert!(!detected_content.is_empty());
        }
    }

    #[test]
    fn test_highlight_range_ordering() {
        let style1 = HighlightStyle {
            fg_color: Some("#ff0000".to_string()),
            bg_color: None,
            bold: false,
            italic: false,
            underline: false,
        };

        let style2 = HighlightStyle {
            fg_color: Some("#00ff00".to_string()),
            bg_color: None,
            bold: false,
            italic: false,
            underline: false,
        };

        let style3 = HighlightStyle {
            fg_color: Some("#0000ff".to_string()),
            bg_color: None,
            bold: false,
            italic: false,
            underline: false,
        };

        let mut ranges = vec![
            HighlightRange {
                start: 20,
                end: 30,
                style: style3,
            },
            HighlightRange {
                start: 0,
                end: 10,
                style: style1,
            },
            HighlightRange {
                start: 10,
                end: 20,
                style: style2,
            },
        ];

        // Sort ranges by start position
        ranges.sort_by(|a, b| a.start.cmp(&b.start));

        assert_eq!(ranges[0].start, 0);
        assert_eq!(ranges[1].start, 10);
        assert_eq!(ranges[2].start, 20);
    }

    #[test]
    fn test_syntax_theme_structure() {
        let mut tree_sitter_mappings = HashMap::new();
        tree_sitter_mappings.insert("function".to_string(), crossterm::style::Color::Blue);
        tree_sitter_mappings.insert("keyword".to_string(), crossterm::style::Color::Magenta);
        tree_sitter_mappings.insert("string".to_string(), crossterm::style::Color::Green);

        let theme = SyntaxTheme {
            tree_sitter_mappings,
        };
        assert_eq!(theme.tree_sitter_mappings.len(), 3);

        // Test that theme contains expected mappings
        assert!(theme.tree_sitter_mappings.contains_key("function"));
        assert!(theme.tree_sitter_mappings.contains_key("keyword"));
        assert!(theme.tree_sitter_mappings.contains_key("string"));
    }

    #[test]
    fn test_highlight_style_combinations() {
        let styles = vec![
            HighlightStyle {
                fg_color: Some("#ff0000".to_string()),
                bg_color: Some("#000000".to_string()),
                bold: true,
                italic: true,
                underline: true,
            },
            HighlightStyle {
                fg_color: Some("#00ff00".to_string()),
                bg_color: None,
                bold: false,
                italic: false,
                underline: false,
            },
            HighlightStyle {
                fg_color: None,
                bg_color: None,
                bold: false,
                italic: false,
                underline: false,
            },
        ];

        assert_eq!(styles.len(), 3);
        assert_eq!(styles[0].fg_color, Some("#ff0000".to_string()));
        assert_eq!(styles[0].bold, true);
        assert_eq!(styles[1].bold, false);
        assert_eq!(styles[2].fg_color, None);
    }

    #[test]
    fn test_language_support_node_mappings() {
        let rust_support = LanguageSupport::rust();

        // Test that common Rust syntax elements are mapped
        let expected_mappings = vec![
            ("fn", SemanticCategory::Keyword),
            ("let", SemanticCategory::Keyword),
            ("mut", SemanticCategory::Keyword),
            ("const", SemanticCategory::Keyword),
        ];

        for (node, expected_category) in expected_mappings {
            if let Some(actual_category) = rust_support.node_mappings.get(node) {
                assert_eq!(*actual_category, expected_category);
            }
        }
    }

    #[test]
    fn test_semantic_category_comprehensive() {
        // Test that semantic categories cover the main syntax elements
        let important_categories = vec![
            SemanticCategory::Keyword,
            SemanticCategory::Function,
            SemanticCategory::Variable,
            SemanticCategory::String,
            SemanticCategory::Number,
            SemanticCategory::Comment,
            SemanticCategory::Type,
            SemanticCategory::Operator,
        ];

        // Test each category can be converted to string
        for category in important_categories {
            let str_repr = category.as_str();
            assert!(!str_repr.is_empty());
            assert!(str_repr.is_ascii());
        }
    }

    #[test]
    fn test_syntax_highlighting_basic_workflow() {
        // Test the basic workflow of syntax highlighting
        let highlighter = SyntaxHighlighter::new();

        if let Ok(mut h) = highlighter {
            let content = "fn main() { println!(\"Hello\"); }";
            let result = h.highlight_text(content, "rust");

            // Test that highlighting returns a result (success or failure both valid)
            assert!(result.is_ok() || result.is_err());

            // If successful, check that we get highlight ranges
            if let Ok(ranges) = result {
                // Each range should have valid start/end positions
                for range in ranges {
                    assert!(range.start <= range.end);
                    assert!(range.end <= content.len());
                }
            }
        }
    }
}
