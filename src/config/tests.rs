use super::*;

#[test]
fn test_language_config_default() {
    let lang_config = LanguageConfig::default();

    // Default should be empty - all config comes from editor.toml
    assert!(lang_config.extensions.is_empty());
    assert!(lang_config.content_patterns.is_empty());
}

#[test]
fn test_language_config_from_file() {
    // Test that we can load language config from the actual editor.toml file
    let config = EditorConfig::load();

    // Should have extensions from editor.toml
    assert_eq!(
        config.languages.detect_language_from_extension("test.rs"),
        Some("rust".to_string())
    );
    assert_eq!(
        config
            .languages
            .detect_language_from_extension("config.toml"),
        Some("toml".to_string())
    );
    assert_eq!(
        config.languages.detect_language_from_extension("readme.md"),
        Some("markdown".to_string())
    );
    assert_eq!(
        config
            .languages
            .detect_language_from_extension("unknown.xyz"),
        None
    );

    // Should have content patterns from editor.toml
    assert_eq!(
        config
            .languages
            .detect_language_from_content("fn main() { let x = 5; }"),
        Some("rust".to_string())
    );
    assert_eq!(
        config
            .languages
            .detect_language_from_content("[package]\nname = \"test\""),
        Some("toml".to_string())
    );
    assert_eq!(
        config
            .languages
            .detect_language_from_content("# Heading\n## Subheading"),
        Some("markdown".to_string())
    );
    assert_eq!(
        config.languages.detect_language_from_content("plain text"),
        None
    );
}

#[test]
fn test_editor_config_has_languages() {
    let config = EditorConfig::load(); // Load from actual file
    assert!(!config.languages.extensions.is_empty());
    assert!(!config.languages.content_patterns.is_empty());
}

#[test]
fn test_language_config_fallbacks() {
    let config = EditorConfig::load();

    // Should have language support
    assert!(config.languages.has_language_support());

    // Should have a fallback language (first configured language)
    assert!(config.languages.get_fallback_language().is_some());

    // Fallback should be one of the configured languages
    let fallback = config.languages.get_fallback_language().unwrap();
    assert!(
        config
            .languages
            .extensions
            .values()
            .any(|lang| lang == &fallback)
    );
}
