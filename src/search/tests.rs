#[cfg(test)]
mod search_tests {
    use super::super::*;

    #[test]
    fn test_search_engine_creation() {
        let engine = SearchEngine::new();
        assert_eq!(engine.last_search, None);
        assert!(!engine.case_sensitive);
        assert!(!engine.use_regex);
    }

    #[test]
    fn test_case_sensitive_search() {
        let mut engine = SearchEngine::new();
        engine.set_case_sensitive(true);

        let text = vec![
            "Hello World".to_string(),
            "hello world".to_string(),
            "HELLO WORLD".to_string(),
        ];

        let results = engine.search("Hello", &text);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].line, 0);
        assert_eq!(results[0].start_col, 0);
        assert_eq!(results[0].end_col, 5);
        assert_eq!(results[0].matched_text, "Hello");
    }

    #[test]
    fn test_case_insensitive_search() {
        let mut engine = SearchEngine::new();
        engine.set_case_sensitive(false);

        let text = vec![
            "Hello World".to_string(),
            "hello world".to_string(),
            "HELLO WORLD".to_string(),
        ];

        let results = engine.search("hello", &text);
        assert_eq!(results.len(), 3);

        // Check first match
        assert_eq!(results[0].line, 0);
        assert_eq!(results[0].start_col, 0);
        assert_eq!(results[0].end_col, 5);
        assert_eq!(results[0].matched_text, "Hello");

        // Check second match
        assert_eq!(results[1].line, 1);
        assert_eq!(results[1].start_col, 0);
        assert_eq!(results[1].end_col, 5);
        assert_eq!(results[1].matched_text, "hello");

        // Check third match
        assert_eq!(results[2].line, 2);
        assert_eq!(results[2].start_col, 0);
        assert_eq!(results[2].end_col, 5);
        assert_eq!(results[2].matched_text, "HELLO");
    }

    #[test]
    fn test_multiple_matches_same_line() {
        let mut engine = SearchEngine::new();

        let text = vec!["test test test".to_string(), "no matches here".to_string()];

        let results = engine.search("test", &text);
        assert_eq!(results.len(), 3);

        assert_eq!(results[0].start_col, 0);
        assert_eq!(results[1].start_col, 5);
        assert_eq!(results[2].start_col, 10);

        for result in &results {
            assert_eq!(result.line, 0);
            assert_eq!(result.matched_text, "test");
        }
    }

    #[test]
    fn test_regex_search() {
        let mut engine = SearchEngine::new();
        engine.set_use_regex(true);

        let text = vec![
            "The quick brown fox".to_string(),
            "jumps over 123 dogs".to_string(),
            "and 456 cats".to_string(),
        ];

        // Search for numbers
        let results = engine.search(r"\d+", &text);
        assert_eq!(results.len(), 2);

        assert_eq!(results[0].line, 1);
        assert_eq!(results[0].matched_text, "123");

        assert_eq!(results[1].line, 2);
        assert_eq!(results[1].matched_text, "456");
    }

    #[test]
    fn test_regex_word_boundaries() {
        let mut engine = SearchEngine::new();
        engine.set_use_regex(true);

        let text = vec![
            "cat catch catastrophe".to_string(),
            "dog doggy dogcatcher".to_string(),
        ];

        // Search for whole word "cat"
        let results = engine.search(r"\bcat\b", &text);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].line, 0);
        assert_eq!(results[0].start_col, 0);
        assert_eq!(results[0].matched_text, "cat");
    }

    #[test]
    fn test_invalid_regex() {
        let mut engine = SearchEngine::new();
        engine.set_use_regex(true);

        let text = vec!["some text".to_string()];

        // Invalid regex should return no results
        let results = engine.search("[", &text);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_empty_search_pattern() {
        let mut engine = SearchEngine::new();

        let text = vec!["some text".to_string()];

        let results = engine.search("", &text);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_search_in_empty_text() {
        let mut engine = SearchEngine::new();

        let text: Vec<String> = vec![];

        let results = engine.search("test", &text);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_search_with_special_characters() {
        let mut engine = SearchEngine::new();

        let text = vec![
            "Hello. World!".to_string(),
            "Test (with) brackets".to_string(),
            "Symbol: @ # $ %".to_string(),
        ];

        // Test punctuation
        let results = engine.search(".", &text);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].line, 0);
        assert_eq!(results[0].matched_text, ".");

        // Test parentheses
        let results = engine.search("(with)", &text);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].line, 1);
        assert_eq!(results[0].matched_text, "(with)");
    }

    #[test]
    fn test_search_result_positions() {
        let mut engine = SearchEngine::new();

        let text = vec![
            "  leading spaces test".to_string(),
            "test trailing spaces  ".to_string(),
        ];

        let results = engine.search("test", &text);
        assert_eq!(results.len(), 2);

        // First match should account for leading spaces
        assert_eq!(results[0].line, 0);
        assert_eq!(results[0].start_col, 17);
        assert_eq!(results[0].end_col, 21);

        // Second match at beginning of line
        assert_eq!(results[1].line, 1);
        assert_eq!(results[1].start_col, 0);
        assert_eq!(results[1].end_col, 4);
    }

    #[test]
    fn test_overlapping_matches() {
        let mut engine = SearchEngine::new();

        let text = vec!["aaaa".to_string()];

        // Should find overlapping matches
        let results = engine.search("aa", &text);
        assert_eq!(results.len(), 3); // positions 0, 1, 2

        assert_eq!(results[0].start_col, 0);
        assert_eq!(results[1].start_col, 1);
        assert_eq!(results[2].start_col, 2);
    }

    #[test]
    fn test_unicode_search() {
        let mut engine = SearchEngine::new();

        let text = vec![
            "Hello 世界".to_string(),
            "café résumé".to_string(),
            "🎉 emoji test 🚀".to_string(),
        ];

        // Test Unicode characters
        let results = engine.search("世界", &text);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].line, 0);
        assert_eq!(results[0].matched_text, "世界");

        // Test accented characters
        let results = engine.search("café", &text);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].line, 1);
        assert_eq!(results[0].matched_text, "café");

        // Test emoji
        let results = engine.search("🎉", &text);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].line, 2);
        assert_eq!(results[0].matched_text, "🎉");
    }

    #[test]
    fn test_last_search_tracking() {
        let mut engine = SearchEngine::new();

        let text = vec!["test".to_string()];

        assert_eq!(engine.last_search, None);

        engine.search("hello", &text);
        assert_eq!(engine.last_search, Some("hello".to_string()));

        engine.search("world", &text);
        assert_eq!(engine.last_search, Some("world".to_string()));
    }

    #[test]
    fn test_search_result_clone() {
        let result = SearchResult {
            line: 5,
            start_col: 10,
            end_col: 15,
            matched_text: "test".to_string(),
        };

        let cloned = result.clone();
        assert_eq!(result.line, cloned.line);
        assert_eq!(result.start_col, cloned.start_col);
        assert_eq!(result.end_col, cloned.end_col);
        assert_eq!(result.matched_text, cloned.matched_text);
    }
}
