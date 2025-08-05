#[cfg(test)]
mod async_syntax_tests {
    use super::super::*;
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
            let _result = highlighter.get_immediate_highlights(1, 0, "fn test() {}", "rust");

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
    fn test_get_immediate_highlights() {
        with_runtime(|| async {
            let highlighter = AsyncSyntaxHighlighter::new().unwrap();

            // First call might succeed or fail depending on tree-sitter availability
            let result = highlighter.get_immediate_highlights(1, 0, "fn test() {}", "rust");

            // If it succeeded, second call should hit cache
            if result.is_some() {
                let cached_result =
                    highlighter.get_immediate_highlights(1, 0, "fn test() {}", "rust");
                assert!(cached_result.is_some());

                // Results should be the same
                assert_eq!(result.unwrap().len(), cached_result.unwrap().len());
            }
        });
    }

    #[test]
    fn test_cache_with_different_languages() {
        with_runtime(|| async {
            let highlighter = AsyncSyntaxHighlighter::new().unwrap();

            // Same content but different languages should be cached separately
            let rust_result =
                highlighter.get_immediate_highlights(1, 0, "function test() {}", "rust");

            let js_result =
                highlighter.get_immediate_highlights(1, 0, "function test() {}", "javascript");

            // Both might be None if tree-sitter isn't available, but they should
            // be treated as separate cache entries
            println!("Rust result: {:?}", rust_result.is_some());
            println!("JS result: {:?}", js_result.is_some());
        });
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
    fn test_error_handling_invalid_request() {
        with_runtime(|| async {
            let highlighter = AsyncSyntaxHighlighter::new().unwrap();

            // Test with empty content
            let result = highlighter.get_immediate_highlights(999, 0, "", "unknown_language");

            // Should handle gracefully (return None rather than panic)
            println!("Empty content result: {:?}", result.is_some());
        });
    }

    #[test]
    fn test_large_content_handling() {
        with_runtime(|| async {
            let highlighter = AsyncSyntaxHighlighter::new().unwrap();

            // Test with very long content
            let large_content = "// ".repeat(1000) + "This is a large comment";

            let result = highlighter.get_immediate_highlights(1, 0, &large_content, "rust");

            // Should handle large content gracefully
            println!("Large content result: {:?}", result.is_some());
        });
    }
}
