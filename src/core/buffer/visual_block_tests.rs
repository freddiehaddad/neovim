#[cfg(test)]
mod visual_block_tests {
    use crate::Position;
    use crate::core::buffer::Buffer;
    use crate::core::mode::Mode;

    fn create_test_buffer() -> Buffer {
        let mut buffer = Buffer::new("test.txt");
        buffer.lines = vec![
            "line one".to_string(),
            "line two longer".to_string(),
            "short".to_string(),
            "another line here".to_string(),
        ];
        buffer.cursor = Position { row: 0, col: 0 };
        buffer
    }

    #[test]
    fn test_visual_block_selection_start() {
        let mut buffer = create_test_buffer();
        buffer.cursor = Position { row: 1, col: 5 };

        // Start visual block selection
        buffer.start_visual_block_selection();

        assert!(buffer.selection.is_some());
        let selection = buffer.selection.as_ref().unwrap();
        assert_eq!(selection.start, Position { row: 1, col: 5 });
        assert_eq!(selection.end, Position { row: 1, col: 5 });
        assert_eq!(buffer.mode, Mode::VisualBlock);
    }

    #[test]
    fn test_visual_block_selection_update() {
        let mut buffer = create_test_buffer();
        buffer.cursor = Position { row: 0, col: 2 };
        buffer.start_visual_block_selection();

        // Move cursor to create a block selection
        buffer.cursor = Position { row: 2, col: 4 };
        buffer.update_visual_selection();

        let selection = buffer.selection.as_ref().unwrap();
        assert_eq!(selection.start, Position { row: 0, col: 2 });
        assert_eq!(selection.end, Position { row: 2, col: 4 });
    }

    #[test]
    fn test_visual_block_yank() {
        let mut buffer = create_test_buffer();
        buffer.cursor = Position { row: 0, col: 5 };
        buffer.start_visual_block_selection();

        // Create a block selection from (0,5) to (2,7)
        buffer.cursor = Position { row: 2, col: 7 };
        buffer.update_visual_selection();

        // Yank the block selection
        buffer.yank_selection();

        // Check clipboard contents
        let expected_text = "one\nlon\n   "; // Visual block with padding
        assert_eq!(buffer.clipboard.text, expected_text);

        // Check that selection is cleared after yank
        assert!(buffer.selection.is_none());
        assert_eq!(buffer.mode, Mode::Normal);
    }

    #[test]
    fn test_visual_block_get_selected_text() {
        let mut buffer = create_test_buffer();
        buffer.cursor = Position { row: 1, col: 0 };
        buffer.start_visual_block_selection();

        // Select block from (1,0) to (3,3)
        buffer.cursor = Position { row: 3, col: 3 };
        buffer.update_visual_selection();

        let selected_text = buffer.get_selected_text();
        let expected = "line\nshor\nano"; // Block selection
        assert_eq!(selected_text, expected);
    }

    #[test]
    fn test_visual_block_get_selected_text_with_padding() {
        let mut buffer = create_test_buffer();
        buffer.cursor = Position { row: 0, col: 6 };
        buffer.start_visual_block_selection();

        // Select block that extends beyond line length
        buffer.cursor = Position { row: 2, col: 8 };
        buffer.update_visual_selection();

        let selected_text = buffer.get_selected_text();
        // Should pad short lines with spaces
        let expected = "one\nlonger\n  "; // "short" line padded
        assert_eq!(selected_text, expected);
    }

    #[test]
    fn test_visual_block_paste_after() {
        let mut buffer = create_test_buffer();

        // Set up clipboard with block text
        buffer.clipboard.text = "AB\nCD\nEF".to_string();
        buffer.clipboard.yank_type = crate::clipboard::YankType::Block;

        // Position cursor and paste
        buffer.cursor = Position { row: 1, col: 4 };
        buffer.put_after();

        // Check that block was inserted correctly
        assert_eq!(buffer.lines[1], "lineAB two longer");
        assert_eq!(buffer.lines[2], "shorCD");
        assert_eq!(buffer.lines[3], "anotEFher line here");
        assert_eq!(buffer.cursor, Position { row: 1, col: 5 });
    }

    #[test]
    fn test_visual_block_paste_before() {
        let mut buffer = create_test_buffer();

        // Set up clipboard with block text
        buffer.clipboard.text = "X\nY\nZ".to_string();
        buffer.clipboard.yank_type = crate::clipboard::YankType::Block;

        // Position cursor and paste
        buffer.cursor = Position { row: 0, col: 5 };
        buffer.put_before();

        // Check that block was inserted correctly
        assert_eq!(buffer.lines[0], "line Xone");
        assert_eq!(buffer.lines[1], "line Ytwo longer");
        assert_eq!(buffer.lines[2], "shortZ");
        assert_eq!(buffer.cursor, Position { row: 0, col: 5 });
    }

    #[test]
    fn test_visual_block_extends_buffer() {
        let mut buffer = create_test_buffer();

        // Set up clipboard with block text that extends beyond buffer
        buffer.clipboard.text = "1\n2\n3\n4\n5".to_string();
        buffer.clipboard.yank_type = crate::clipboard::YankType::Block;

        // Position cursor near end of buffer and paste
        buffer.cursor = Position { row: 3, col: 0 };
        buffer.put_after();

        // Check that buffer was extended with new lines
        assert_eq!(buffer.lines.len(), 8); // Original 4 + 4 new lines
        assert_eq!(buffer.lines[4], "1");
        assert_eq!(buffer.lines[5], "2");
        assert_eq!(buffer.lines[6], "3");
        assert_eq!(buffer.lines[7], "4");
    }

    #[test]
    fn test_visual_block_clear_selection() {
        let mut buffer = create_test_buffer();
        buffer.cursor = Position { row: 0, col: 0 };
        buffer.start_visual_block_selection();

        assert!(buffer.selection.is_some());
        assert_eq!(buffer.mode, Mode::VisualBlock);

        buffer.clear_selection();

        assert!(buffer.selection.is_none());
        assert_eq!(buffer.mode, Mode::Normal);
    }

    #[test]
    fn test_visual_block_single_character() {
        let mut buffer = create_test_buffer();
        buffer.cursor = Position { row: 1, col: 5 };
        buffer.start_visual_block_selection();

        // Yank single character block
        buffer.yank_selection();

        // Should contain just the character under cursor
        assert_eq!(buffer.clipboard.text, "t");
    }

    #[test]
    fn test_visual_block_empty_lines() {
        let mut buffer = Buffer::new("empty_test.txt");
        buffer.lines = vec!["text".to_string(), "".to_string(), "more".to_string()];
        buffer.cursor = Position { row: 0, col: 1 };
        buffer.start_visual_block_selection();

        // Select block that includes empty line
        buffer.cursor = Position { row: 2, col: 3 };
        buffer.update_visual_selection();

        let selected_text = buffer.get_selected_text();
        let expected = "ext\n   \nore"; // Empty line padded with spaces
        assert_eq!(selected_text, expected);
    }
}
