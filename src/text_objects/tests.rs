#[cfg(test)]
mod text_objects_tests {
    use super::super::*;
    use crate::buffer::Buffer;
    use crate::mode::Position;

    fn create_buffer_with_content(content: &str) -> Buffer {
        let mut buffer = Buffer::new(1, 100); // id=1, undo_levels=100
        buffer.lines.clear();
        for line in content.lines() {
            buffer.lines.push(line.to_string());
        }
        if buffer.lines.is_empty() {
            buffer.lines.push(String::new());
        }
        buffer
    }

    #[test]
    fn test_parse_text_object_word() {
        assert_eq!(
            parse_text_object("iw"),
            Some((TextObjectMode::Inner, TextObjectType::Word))
        );
        assert_eq!(
            parse_text_object("aw"),
            Some((TextObjectMode::Around, TextObjectType::Word))
        );
        assert_eq!(
            parse_text_object("iW"),
            Some((TextObjectMode::Inner, TextObjectType::Word2))
        );
        assert_eq!(
            parse_text_object("aW"),
            Some((TextObjectMode::Around, TextObjectType::Word2))
        );
    }

    #[test]
    fn test_parse_text_object_paragraph() {
        assert_eq!(
            parse_text_object("ip"),
            Some((TextObjectMode::Inner, TextObjectType::Paragraph))
        );
        assert_eq!(
            parse_text_object("ap"),
            Some((TextObjectMode::Around, TextObjectType::Paragraph))
        );
    }

    #[test]
    fn test_parse_text_object_sentence() {
        assert_eq!(
            parse_text_object("is"),
            Some((TextObjectMode::Inner, TextObjectType::Sentence))
        );
        assert_eq!(
            parse_text_object("as"),
            Some((TextObjectMode::Around, TextObjectType::Sentence))
        );
    }

    #[test]
    fn test_parse_text_object_quotes() {
        assert_eq!(
            parse_text_object("i\""),
            Some((TextObjectMode::Inner, TextObjectType::Quote))
        );
        assert_eq!(
            parse_text_object("a\""),
            Some((TextObjectMode::Around, TextObjectType::Quote))
        );
        assert_eq!(
            parse_text_object("i'"),
            Some((TextObjectMode::Inner, TextObjectType::Quote))
        );
        assert_eq!(
            parse_text_object("a'"),
            Some((TextObjectMode::Around, TextObjectType::Quote))
        );
        assert_eq!(
            parse_text_object("i`"),
            Some((TextObjectMode::Inner, TextObjectType::Quote))
        );
        assert_eq!(
            parse_text_object("a`"),
            Some((TextObjectMode::Around, TextObjectType::Quote))
        );
    }

    #[test]
    fn test_parse_text_object_brackets() {
        assert_eq!(
            parse_text_object("i("),
            Some((TextObjectMode::Inner, TextObjectType::Paren))
        );
        assert_eq!(
            parse_text_object("a("),
            Some((TextObjectMode::Around, TextObjectType::Paren))
        );
        assert_eq!(
            parse_text_object("i["),
            Some((TextObjectMode::Inner, TextObjectType::Bracket))
        );
        assert_eq!(
            parse_text_object("a["),
            Some((TextObjectMode::Around, TextObjectType::Bracket))
        );
        assert_eq!(
            parse_text_object("i{"),
            Some((TextObjectMode::Inner, TextObjectType::Brace))
        );
        assert_eq!(
            parse_text_object("a{"),
            Some((TextObjectMode::Around, TextObjectType::Brace))
        );
        assert_eq!(
            parse_text_object("i<"),
            Some((TextObjectMode::Inner, TextObjectType::Angle))
        );
        assert_eq!(
            parse_text_object("a<"),
            Some((TextObjectMode::Around, TextObjectType::Angle))
        );
    }

    #[test]
    fn test_parse_text_object_tags() {
        assert_eq!(
            parse_text_object("it"),
            Some((TextObjectMode::Inner, TextObjectType::Tag))
        );
        assert_eq!(
            parse_text_object("at"),
            Some((TextObjectMode::Around, TextObjectType::Tag))
        );
    }

    #[test]
    fn test_parse_text_object_invalid() {
        assert_eq!(parse_text_object("ix"), None);
        assert_eq!(parse_text_object(""), None);
        assert_eq!(parse_text_object("xyz"), None);
    }

    #[test]
    fn test_find_word_inner() {
        let buffer = create_buffer_with_content("hello world test");
        let finder = TextObjectFinder::new();

        // Cursor on "world"
        let cursor = Position { row: 0, col: 8 };
        let result = finder
            .find_text_object(&buffer, cursor, TextObjectType::Word, TextObjectMode::Inner)
            .unwrap();

        assert!(result.is_some());
        let range = result.unwrap();
        assert_eq!(range.start, Position { row: 0, col: 6 });
        assert_eq!(range.end, Position { row: 0, col: 11 });
        assert_eq!(range.get_text(&buffer), "world");
    }

    #[test]
    fn test_find_word_around() {
        let buffer = create_buffer_with_content("hello world test");
        let finder = TextObjectFinder::new();

        // Cursor on "world"
        let cursor = Position { row: 0, col: 8 };
        let result = finder
            .find_text_object(
                &buffer,
                cursor,
                TextObjectType::Word,
                TextObjectMode::Around,
            )
            .unwrap();

        assert!(result.is_some());
        let range = result.unwrap();
        assert_eq!(range.start, Position { row: 0, col: 6 }); // Start of "world"
        assert_eq!(range.end, Position { row: 0, col: 12 }); // Include trailing space before "test"
        assert_eq!(range.get_text(&buffer), "world ");
    }

    #[test]
    fn test_find_word_at_beginning() {
        let buffer = create_buffer_with_content("hello world");
        let finder = TextObjectFinder::new();

        // Cursor on "hello"
        let cursor = Position { row: 0, col: 2 };
        let result = finder
            .find_text_object(&buffer, cursor, TextObjectType::Word, TextObjectMode::Inner)
            .unwrap();

        assert!(result.is_some());
        let range = result.unwrap();
        assert_eq!(range.start, Position { row: 0, col: 0 });
        assert_eq!(range.end, Position { row: 0, col: 5 });
        assert_eq!(range.get_text(&buffer), "hello");
    }

    #[test]
    fn test_find_word_at_end() {
        let buffer = create_buffer_with_content("hello world");
        let finder = TextObjectFinder::new();

        // Cursor on "world" (last word, so should include leading space)
        let cursor = Position { row: 0, col: 9 };
        let result = finder
            .find_text_object(
                &buffer,
                cursor,
                TextObjectType::Word,
                TextObjectMode::Around,
            )
            .unwrap();

        assert!(result.is_some());
        let range = result.unwrap();
        assert_eq!(range.start, Position { row: 0, col: 5 }); // Include leading space
        assert_eq!(range.end, Position { row: 0, col: 11 });
        assert_eq!(range.get_text(&buffer), " world");
    }

    #[test]
    fn test_find_paragraph_inner() {
        let buffer = create_buffer_with_content(
            "First paragraph line 1\nFirst paragraph line 2\n\nSecond paragraph line 1\nSecond paragraph line 2",
        );
        let finder = TextObjectFinder::new();

        // Cursor in first paragraph
        let cursor = Position { row: 1, col: 5 };
        let result = finder
            .find_text_object(
                &buffer,
                cursor,
                TextObjectType::Paragraph,
                TextObjectMode::Inner,
            )
            .unwrap();

        assert!(result.is_some());
        let range = result.unwrap();
        assert_eq!(range.start, Position { row: 0, col: 0 });
        assert_eq!(range.end, Position { row: 2, col: 0 }); // Up to but not including blank line
    }

    #[test]
    fn test_find_paragraph_around() {
        let buffer = create_buffer_with_content(
            "First paragraph line 1\nFirst paragraph line 2\n\nSecond paragraph line 1",
        );
        let finder = TextObjectFinder::new();

        // Cursor in first paragraph
        let cursor = Position { row: 0, col: 5 };
        let result = finder
            .find_text_object(
                &buffer,
                cursor,
                TextObjectType::Paragraph,
                TextObjectMode::Around,
            )
            .unwrap();

        assert!(result.is_some());
        let range = result.unwrap();
        assert_eq!(range.start, Position { row: 0, col: 0 });
        assert_eq!(range.end, Position { row: 3, col: 0 }); // Include trailing blank line
    }

    #[test]
    fn test_find_quotes_inner() {
        let buffer = create_buffer_with_content("Say \"hello world\" to everyone");
        let finder = TextObjectFinder::new();

        // Cursor inside quotes
        let cursor = Position { row: 0, col: 10 };
        let result = finder
            .find_text_object(
                &buffer,
                cursor,
                TextObjectType::Quote,
                TextObjectMode::Inner,
            )
            .unwrap();

        assert!(result.is_some());
        let range = result.unwrap();
        assert_eq!(range.start, Position { row: 0, col: 5 });
        assert_eq!(range.end, Position { row: 0, col: 16 });
        assert_eq!(range.get_text(&buffer), "hello world");
    }

    #[test]
    fn test_find_quotes_around() {
        let buffer = create_buffer_with_content("Say \"hello world\" to everyone");
        let finder = TextObjectFinder::new();

        // Cursor inside quotes
        let cursor = Position { row: 0, col: 10 };
        let result = finder
            .find_text_object(
                &buffer,
                cursor,
                TextObjectType::Quote,
                TextObjectMode::Around,
            )
            .unwrap();

        assert!(result.is_some());
        let range = result.unwrap();
        assert_eq!(range.start, Position { row: 0, col: 4 });
        assert_eq!(range.end, Position { row: 0, col: 17 });
        assert_eq!(range.get_text(&buffer), "\"hello world\"");
    }

    #[test]
    fn test_find_parentheses_inner() {
        let buffer = create_buffer_with_content("function(arg1, arg2)");
        let finder = TextObjectFinder::new();

        // Cursor inside parentheses
        let cursor = Position { row: 0, col: 12 };
        let result = finder
            .find_text_object(
                &buffer,
                cursor,
                TextObjectType::Paren,
                TextObjectMode::Inner,
            )
            .unwrap();

        assert!(result.is_some());
        let range = result.unwrap();
        assert_eq!(range.start, Position { row: 0, col: 9 });
        assert_eq!(range.end, Position { row: 0, col: 19 });
        assert_eq!(range.get_text(&buffer), "arg1, arg2");
    }

    #[test]
    fn test_find_parentheses_around() {
        let buffer = create_buffer_with_content("function(arg1, arg2)");
        let finder = TextObjectFinder::new();

        // Cursor inside parentheses
        let cursor = Position { row: 0, col: 12 };
        let result = finder
            .find_text_object(
                &buffer,
                cursor,
                TextObjectType::Paren,
                TextObjectMode::Around,
            )
            .unwrap();

        assert!(result.is_some());
        let range = result.unwrap();
        assert_eq!(range.start, Position { row: 0, col: 8 });
        assert_eq!(range.end, Position { row: 0, col: 20 });
        assert_eq!(range.get_text(&buffer), "(arg1, arg2)");
    }

    #[test]
    fn test_find_nested_brackets() {
        let buffer = create_buffer_with_content("outer [inner [nested] content] end");
        let finder = TextObjectFinder::new();

        // Cursor in inner bracket content
        let cursor = Position { row: 0, col: 15 }; // On "nested"
        let result = finder
            .find_text_object(
                &buffer,
                cursor,
                TextObjectType::Bracket,
                TextObjectMode::Inner,
            )
            .unwrap();

        assert!(result.is_some());
        let range = result.unwrap();
        assert_eq!(range.start, Position { row: 0, col: 14 });
        assert_eq!(range.end, Position { row: 0, col: 20 });
        assert_eq!(range.get_text(&buffer), "nested");
    }

    #[test]
    fn test_find_html_tag_inner() {
        let buffer = create_buffer_with_content("<div>content here</div>");
        let finder = TextObjectFinder::new();

        // Cursor inside tag
        let cursor = Position { row: 0, col: 8 };
        let result = finder
            .find_text_object(&buffer, cursor, TextObjectType::Tag, TextObjectMode::Inner)
            .unwrap();

        // HTML tag parsing is not yet implemented - should return None
        assert!(result.is_none());
    }

    #[test]
    fn test_find_html_tag_around() {
        let buffer = create_buffer_with_content("<div>content here</div>");
        let finder = TextObjectFinder::new();

        // Cursor inside tag
        let cursor = Position { row: 0, col: 8 };
        let result = finder
            .find_text_object(&buffer, cursor, TextObjectType::Tag, TextObjectMode::Around)
            .unwrap();

        // HTML tag parsing is not yet implemented - should return None
        assert!(result.is_none());
    }

    #[test]
    fn test_find_sentence() {
        let buffer = create_buffer_with_content("First sentence. Second sentence! Third sentence?");
        let finder = TextObjectFinder::new();

        // Cursor in second sentence
        let cursor = Position { row: 0, col: 20 };
        let result = finder
            .find_text_object(
                &buffer,
                cursor,
                TextObjectType::Sentence,
                TextObjectMode::Inner,
            )
            .unwrap();

        assert!(result.is_some());
        let range = result.unwrap();
        assert_eq!(range.start, Position { row: 0, col: 16 });
        assert_eq!(range.end, Position { row: 0, col: 32 });
        assert_eq!(range.get_text(&buffer), "Second sentence!");
    }

    #[test]
    fn test_text_object_range_get_text() {
        let buffer = create_buffer_with_content("line1\nline2\nline3");

        let range = TextObjectRange {
            start: Position { row: 0, col: 2 },
            end: Position { row: 1, col: 3 },
            object_type: TextObjectType::Word,
            mode: TextObjectMode::Inner,
        };

        let text = range.get_text(&buffer);
        assert_eq!(text, "ne1\nlin");
    }

    #[test]
    fn test_find_word_on_punctuation() {
        let buffer = create_buffer_with_content("hello, world!");
        let finder = TextObjectFinder::new();

        // Cursor on comma - both inner and around mode should return None (correct vim behavior)
        let cursor = Position { row: 0, col: 5 };
        let result = finder
            .find_text_object(&buffer, cursor, TextObjectType::Word, TextObjectMode::Inner)
            .unwrap();
        assert!(result.is_none());

        let result = finder
            .find_text_object(
                &buffer,
                cursor,
                TextObjectType::Word,
                TextObjectMode::Around,
            )
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_find_word2_with_punctuation() {
        let buffer = create_buffer_with_content("hello-world test_case");
        let finder = TextObjectFinder::new();

        // Cursor on "hello-world" (should be treated as one WORD)
        let cursor = Position { row: 0, col: 8 };
        let result = finder
            .find_text_object(
                &buffer,
                cursor,
                TextObjectType::Word2,
                TextObjectMode::Inner,
            )
            .unwrap();

        assert!(result.is_some());
        let range = result.unwrap();
        assert_eq!(range.start, Position { row: 0, col: 0 });
        assert_eq!(range.end, Position { row: 0, col: 11 });
        assert_eq!(range.get_text(&buffer), "hello-world");
    }

    #[test]
    fn test_empty_text_object() {
        let buffer = create_buffer_with_content("");
        let finder = TextObjectFinder::new();

        let cursor = Position { row: 0, col: 0 };
        let result = finder
            .find_text_object(&buffer, cursor, TextObjectType::Word, TextObjectMode::Inner)
            .unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn test_cursor_out_of_bounds() {
        let buffer = create_buffer_with_content("hello");
        let finder = TextObjectFinder::new();

        // Cursor beyond line end
        let cursor = Position { row: 0, col: 100 };
        let result =
            finder.find_text_object(&buffer, cursor, TextObjectType::Word, TextObjectMode::Inner);

        // Should handle gracefully
        assert!(result.is_ok());
    }
}
