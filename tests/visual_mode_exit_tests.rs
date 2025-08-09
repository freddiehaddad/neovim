//! Integration tests for visual mode exit behavior
//! These tests verify that visual selection is properly cleared when exiting visual mode

use oxidized::core::{buffer::Buffer, mode::Position};

#[cfg(test)]
mod visual_mode_exit_tests {
    use super::*;

    /// Test that yank_selection clears the selection (matches Vim behavior)
    #[test]
    fn test_yank_selection_clears_selection() {
        let mut buffer = Buffer::new(1, 100);

        // Insert test text
        let test_text = "Hello world test";
        for ch in test_text.chars() {
            buffer.insert_char(ch);
        }
        buffer.cursor = Position::new(0, 0); // Reset to start

        // Start visual selection
        buffer.start_visual_selection();
        assert!(buffer.has_selection());

        // Move cursor to select "Hello"
        buffer.cursor = Position::new(0, 5);
        buffer.update_visual_selection(Position::new(0, 5));

        // Verify we have the expected selection
        let selected_text = buffer.get_selected_text().unwrap();
        assert_eq!(selected_text, "Hello");

        // Yank the selection - this should clear the selection after yanking
        let yanked_text = buffer.yank_selection().unwrap();
        assert_eq!(yanked_text, "Hello");

        // CRITICAL TEST: Selection should be cleared after yanking
        assert!(
            !buffer.has_selection(),
            "Selection should be cleared after yanking"
        );

        // Verify clipboard contains the yanked text
        assert_eq!(buffer.clipboard.text, "Hello");
    }

    /// Test that delete_selection clears the selection (this was already working)
    #[test]
    fn test_delete_selection_clears_selection() {
        let mut buffer = Buffer::new(1, 100);

        // Insert test text
        let test_text = "Hello world test";
        for ch in test_text.chars() {
            buffer.insert_char(ch);
        }
        buffer.cursor = Position::new(0, 6); // Position at "w"

        // Start visual selection
        buffer.start_visual_selection();

        // Move cursor to select "world"
        buffer.cursor = Position::new(0, 11);
        buffer.update_visual_selection(Position::new(0, 11));

        // Verify we have the expected selection
        let selected_text = buffer.get_selected_text().unwrap();
        assert_eq!(selected_text, "world");

        // Delete the selection - this should clear the selection
        let deleted_text = buffer.delete_selection().unwrap();
        assert_eq!(deleted_text, "world");

        // Selection should be cleared after deleting
        assert!(
            !buffer.has_selection(),
            "Selection should be cleared after deleting"
        );
    }

    /// Test that clear_visual_selection works correctly
    #[test]
    fn test_clear_visual_selection_works() {
        let mut buffer = Buffer::new(1, 100);

        // Insert test text
        let test_text = "Test text for selection";
        for ch in test_text.chars() {
            buffer.insert_char(ch);
        }
        buffer.cursor = Position::new(0, 5);

        // Start visual selection
        buffer.start_visual_selection();
        assert!(buffer.has_selection());

        // Move cursor to extend selection
        buffer.cursor = Position::new(0, 9);
        buffer.update_visual_selection(Position::new(0, 9));

        // Verify we have a selection
        assert!(buffer.has_selection());
        let selected_text = buffer.get_selected_text().unwrap();
        assert_eq!(selected_text, "text");

        // Clear the selection
        buffer.clear_visual_selection();

        // Selection should be gone
        assert!(!buffer.has_selection(), "Selection should be cleared");
    }
}
