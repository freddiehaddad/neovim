use oxidized::core::{
    buffer::Buffer,
    buffer::{ClipboardContent, YankType},
    mode::Position,
};

#[cfg(test)]
mod empty_line_paste_tests {
    use super::*;

    #[test]
    fn test_paste_multiline_text_with_empty_line() {
        // This test reproduces the panic scenario where clipboard contains text with empty lines
        let mut buffer = Buffer::new(1, 100);

        // Insert test content manually
        let lines = ["First line", "Second line", "Third line"];
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                buffer.insert_line_break(); // Add newline before each line except first
            }
            for ch in line.chars() {
                buffer.insert_char(ch);
            }
        }

        // Move cursor to end of third line
        buffer.cursor = Position { row: 2, col: 10 };

        // Simulate a paste operation with content that includes an empty line at the end
        // This is what would happen when copying "README content\n\n" from a file
        let clipboard_content = ClipboardContent {
            text: "README content\n".to_string(), // This ends with \n which creates empty line
            yank_type: YankType::Character,
        };
        buffer.clipboard = clipboard_content;

        // This should not panic even though the last line after split is empty
        buffer.put_after();

        // Verify the buffer state is correct
        assert_eq!(buffer.lines.len(), 4);
        assert_eq!(buffer.lines[2], "Third lineREADME content");
        assert_eq!(buffer.lines[3], "");

        // Verify cursor position is safe (no underflow)
        assert_eq!(buffer.cursor.row, 3);
        assert_eq!(buffer.cursor.col, 0); // Should be 0 for empty line, not underflowed
    }

    #[test]
    fn test_paste_multiline_text_ending_with_double_newline() {
        // Test the exact scenario from the log: content ending with \n\n
        let mut buffer = Buffer::new(1, 100);

        // Insert single line
        for ch in "Line one".chars() {
            buffer.insert_char(ch);
        }

        buffer.cursor = Position { row: 0, col: 8 }; // End of first line

        // Content with double newline at end (like README file)
        let clipboard_content = ClipboardContent {
            text: "Some text\n\n".to_string(), // Double newline creates empty line
            yank_type: YankType::Character,
        };
        buffer.clipboard = clipboard_content;

        // This should not panic
        buffer.put_after();

        // Verify the buffer state
        assert_eq!(buffer.lines.len(), 3);
        assert_eq!(buffer.lines[0], "Line oneSome text");
        assert_eq!(buffer.lines[1], "");
        assert_eq!(buffer.lines[2], "");

        // Cursor should be positioned safely at start of last empty line
        assert_eq!(buffer.cursor.row, 2);
        assert_eq!(buffer.cursor.col, 0); // No underflow panic
    }

    #[test]
    fn test_put_before_with_empty_line() {
        // Test put_before method with same edge case
        let mut buffer = Buffer::new(1, 100);

        // Insert single line
        for ch in "Original line".chars() {
            buffer.insert_char(ch);
        }

        buffer.cursor = Position { row: 0, col: 5 }; // Middle of line

        let clipboard_content = ClipboardContent {
            text: "Inserted\n".to_string(), // Creates empty line
            yank_type: YankType::Character,
        };
        buffer.clipboard = clipboard_content;

        // This should also not panic
        buffer.put_before();

        // Verify no panic and correct positioning
        assert_eq!(buffer.cursor.col, 0); // Should be 0, not underflowed
    }

    #[test]
    fn test_paste_completely_empty_clipboard() {
        // Test edge case: completely empty clipboard content
        let mut buffer = Buffer::new(1, 100);

        // Insert single line
        for ch in "Test line".chars() {
            buffer.insert_char(ch);
        }

        buffer.cursor = Position { row: 0, col: 4 }; // Middle of line

        let clipboard_content = ClipboardContent {
            text: "".to_string(), // Completely empty string
            yank_type: YankType::Character,
        };
        buffer.clipboard = clipboard_content;

        // This should not panic even with empty clipboard
        buffer.put_after();

        // Verify buffer state is unchanged (since nothing to paste)
        assert_eq!(buffer.lines.len(), 1);
        assert_eq!(buffer.lines[0], "Test line");

        // Cursor should not move for empty paste
        assert_eq!(buffer.cursor.row, 0);
        assert_eq!(buffer.cursor.col, 4); // Should remain at original position
    }
}
