#[cfg(test)]
mod buffer_tests {
    use super::super::*;

    fn create_test_buffer() -> Buffer {
        Buffer::new(1, 100) // id=1, undo_levels=100
    }

    fn create_buffer_with_content(content: &str) -> Buffer {
        let mut buffer = create_test_buffer();
        buffer.lines.clear(); // Clear the initial empty line
        for line in content.lines() {
            buffer.lines.push(line.to_string());
        }
        if buffer.lines.is_empty() {
            buffer.lines.push(String::new());
        }
        buffer
    }

    #[test]
    fn test_buffer_creation() {
        let buffer = create_test_buffer();
        assert_eq!(buffer.lines.len(), 1);
        assert_eq!(buffer.lines[0], "");
        assert_eq!(buffer.cursor.row, 0);
        assert_eq!(buffer.cursor.col, 0);
        assert!(!buffer.modified);
    }

    #[test]
    fn test_insert_char() {
        let mut buffer = create_test_buffer();
        buffer.insert_char('h');
        buffer.insert_char('i');

        assert_eq!(buffer.lines[0], "hi");
        assert_eq!(buffer.cursor.col, 2);
        assert!(buffer.modified);
    }

    #[test]
    fn test_insert_char_middle_of_line() {
        let mut buffer = create_buffer_with_content("hello");
        buffer.cursor.col = 2;
        buffer.insert_char('X');

        assert_eq!(buffer.lines[0], "heXllo");
        assert_eq!(buffer.cursor.col, 3);
    }

    #[test]
    fn test_delete_char() {
        let mut buffer = create_buffer_with_content("hello");
        buffer.cursor.col = 4;
        buffer.delete_char();

        // delete_char() deletes the character BEFORE cursor, so 'l' at position 3
        assert_eq!(buffer.lines[0], "helo");
        assert_eq!(buffer.cursor.col, 3);
    }

    #[test]
    fn test_delete_char_at_cursor() {
        let mut buffer = create_buffer_with_content("hello");
        buffer.cursor.col = 2;
        buffer.delete_char_at_cursor();

        assert_eq!(buffer.lines[0], "helo");
        assert_eq!(buffer.cursor.col, 2);
    }

    #[test]
    fn test_delete_char_before_cursor() {
        let mut buffer = create_buffer_with_content("hello");
        buffer.cursor.col = 2;
        buffer.delete_char_before_cursor();

        assert_eq!(buffer.lines[0], "hllo");
        assert_eq!(buffer.cursor.col, 1);
    }

    #[test]
    fn test_insert_line_break() {
        let mut buffer = create_buffer_with_content("hello");
        buffer.cursor.col = 2;
        buffer.insert_line_break();

        assert_eq!(buffer.lines.len(), 2);
        assert_eq!(buffer.lines[0], "he");
        assert_eq!(buffer.lines[1], "llo");
        assert_eq!(buffer.cursor.row, 1);
        assert_eq!(buffer.cursor.col, 0);
    }

    #[test]
    fn test_delete_line() {
        let mut buffer = create_buffer_with_content("line1\nline2\nline3");
        buffer.cursor.row = 1;
        buffer.delete_line();

        assert_eq!(buffer.lines.len(), 2);
        assert_eq!(buffer.lines[0], "line1");
        assert_eq!(buffer.lines[1], "line3");
        assert_eq!(buffer.cursor.row, 1);
    }

    #[test]
    fn test_delete_last_line() {
        let mut buffer = create_buffer_with_content("line1\nline2");
        buffer.cursor.row = 1;
        buffer.delete_line();

        assert_eq!(buffer.lines.len(), 1);
        assert_eq!(buffer.lines[0], "line1");
        assert_eq!(buffer.cursor.row, 0);
    }

    #[test]
    fn test_move_cursor_bounds() {
        let mut buffer = create_buffer_with_content("short\nlonger line");

        // Test moving down and column adjustment
        buffer.cursor.col = 4;
        assert_eq!(buffer.cursor.col, 4); // Within "short"

        // Move to second line
        buffer.cursor.row = 1;
        buffer.cursor.col = buffer.cursor.col.min(buffer.lines[1].len());
        assert_eq!(buffer.cursor.col, 4); // Should stay at 4 in "longer line"

        // Move back to first line
        buffer.cursor.row = 0;
        buffer.cursor.col = buffer.cursor.col.min(buffer.lines[0].len());
        assert_eq!(buffer.cursor.col, 4); // Should stay within "short" bounds
    }

    #[test]
    fn test_word_movement() {
        let mut buffer = create_buffer_with_content("hello world test");

        // Move to next word
        buffer.move_to_next_word();
        assert_eq!(buffer.cursor.col, 6); // Start of "world"

        buffer.move_to_next_word();
        assert_eq!(buffer.cursor.col, 12); // Start of "test"

        // Move to previous word
        buffer.move_to_previous_word();
        assert_eq!(buffer.cursor.col, 6); // Back to "world"

        buffer.move_to_previous_word();
        assert_eq!(buffer.cursor.col, 0); // Back to "hello"
    }

    #[test]
    fn test_word_end_movement() {
        let mut buffer = create_buffer_with_content("hello world");

        buffer.move_to_word_end();
        assert_eq!(buffer.cursor.col, 4); // End of "hello"

        buffer.cursor.col = 6; // Start of "world"
        buffer.move_to_word_end();
        assert_eq!(buffer.cursor.col, 10); // End of "world"
    }

    #[test]
    fn test_yank_operations() {
        let mut buffer = create_buffer_with_content("hello world\nsecond line");

        // Test yank line (should include newline in vim-compatible behavior)
        buffer.yank_line();
        assert_eq!(buffer.clipboard.text, "hello world\n");
        assert_eq!(buffer.clipboard.yank_type, YankType::Line);

        // Test yank word
        buffer.cursor.col = 0;
        buffer.yank_word();
        assert_eq!(buffer.clipboard.text, "hello");
        assert_eq!(buffer.clipboard.yank_type, YankType::Character);

        // Test yank to end of line
        buffer.cursor.col = 6;
        buffer.yank_to_end_of_line();
        assert_eq!(buffer.clipboard.text, "world");
        assert_eq!(buffer.clipboard.yank_type, YankType::Character);
    }

    #[test]
    fn test_put_operations() {
        let mut buffer = create_buffer_with_content("hello");

        // Set up clipboard with character yank
        buffer.clipboard.text = "world".to_string();
        buffer.clipboard.yank_type = YankType::Character;

        // Put after cursor
        buffer.cursor.col = 5;
        buffer.put_after();
        assert_eq!(buffer.lines[0], "helloworld");
        assert_eq!(buffer.cursor.col, 9);

        // Put before cursor
        buffer.clipboard.text = "X".to_string();
        buffer.cursor.col = 5;
        buffer.put_before();
        assert_eq!(buffer.lines[0], "helloXworld");
        assert_eq!(buffer.cursor.col, 5);
    }

    #[test]
    fn test_put_line_operations() {
        let mut buffer = create_buffer_with_content("line1\nline2");

        // Set up clipboard with line yank (without trailing newline for the line content)
        buffer.clipboard.text = "inserted line".to_string();
        buffer.clipboard.yank_type = YankType::Line;

        // Put after current line
        buffer.cursor.row = 0;
        buffer.put_after();
        assert_eq!(buffer.lines.len(), 3);
        assert_eq!(buffer.lines[0], "line1");
        assert_eq!(buffer.lines[1], "inserted line");
        assert_eq!(buffer.lines[2], "line2");
        assert_eq!(buffer.cursor.row, 1);

        // Put before current line
        buffer.clipboard.text = "another line".to_string();
        buffer.cursor.row = 1;
        buffer.put_before();
        assert_eq!(buffer.lines.len(), 4);
        assert_eq!(buffer.lines[0], "line1");
        assert_eq!(buffer.lines[1], "another line");
        assert_eq!(buffer.lines[2], "inserted line");
        assert_eq!(buffer.lines[3], "line2");
        assert_eq!(buffer.cursor.row, 1);
    }

    #[test]
    fn test_undo_redo_basic() {
        let mut buffer = create_test_buffer();

        // Test that undo/redo functions exist and can be called
        let undo_result = buffer.undo();
        assert_eq!(undo_result, false); // No operations to undo

        let redo_result = buffer.redo();
        assert_eq!(redo_result, false); // No operations to redo

        // Insert a character using the proper method
        buffer.insert_char('h');
        assert_eq!(buffer.lines[0], "h");
        assert_eq!(buffer.cursor.col, 1);

        // Undo the insertion
        let undo_result = buffer.undo();
        assert_eq!(undo_result, true); // Should succeed
        assert_eq!(buffer.lines[0], "");
        assert_eq!(buffer.cursor.col, 0);
    }

    #[test]
    fn test_undo_redo_delete() {
        let mut buffer = create_buffer_with_content("hello world");
        buffer.cursor.col = 6;

        // Delete "world"
        let deleted_text = "world".to_string();
        buffer.save_operation(EditOperation::Delete {
            pos: Position { row: 0, col: 6 },
            text: deleted_text,
        });
        buffer.lines[0] = "hello ".to_string();

        // Undo delete
        buffer.undo();
        assert_eq!(buffer.lines[0], "hello world");
        assert_eq!(buffer.cursor.col, 6);

        // Redo delete
        buffer.redo();
        assert_eq!(buffer.lines[0], "hello ");
        assert_eq!(buffer.cursor.col, 6);
    }

    #[test]
    fn test_range_operations() {
        let mut buffer = create_buffer_with_content("line1\nline2\nline3\nline4");

        let start = Position { row: 1, col: 0 };
        let end = Position { row: 2, col: 5 };

        // Test delete range - deleting "line2\nline3"
        let deleted_text = buffer.delete_range(start, end);

        // The deletion removes lines 1 and 2, and replaces them with an empty line
        // This is because we're deleting from start of line2 to end of line3
        assert_eq!(buffer.lines.len(), 3);
        assert_eq!(buffer.lines[0], "line1");
        assert_eq!(buffer.lines[1], ""); // Empty line where line2/line3 were joined
        assert_eq!(buffer.lines[2], "line4");
        assert_eq!(deleted_text, "line2\nline3");

        // Undo the range deletion
        buffer.undo();
        assert_eq!(buffer.lines.len(), 4);
        assert_eq!(buffer.lines[1], "line2");
        assert_eq!(buffer.lines[2], "line3");
    }

    #[test]
    fn test_replace_range_with_undo() {
        let mut buffer = create_buffer_with_content("hello world test");

        let start = Position { row: 0, col: 6 };
        let end = Position { row: 0, col: 11 };

        // Replace "world" with "REPLACEMENT" (replace_range handles undo automatically)
        buffer.replace_range(start, end, "REPLACEMENT");
        assert_eq!(buffer.lines[0], "hello REPLACEMENT test");

        // Undo the replacement
        buffer.undo();
        assert_eq!(buffer.lines[0], "hello world test");

        // Redo the replacement
        buffer.redo();
        assert_eq!(buffer.lines[0], "hello REPLACEMENT test");
    }
}
