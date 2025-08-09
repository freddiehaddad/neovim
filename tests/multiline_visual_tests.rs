use oxidized::core::{buffer::Buffer, mode::Position};

#[cfg(test)]
mod multiline_visual_tests {
    use super::*;

    /// Test multi-line visual selection yank and paste behavior
    #[test]
    fn test_multiline_visual_selection_yank_paste() {
        let mut buffer = Buffer::new(1, 100);
        
        // Insert multi-line test text
        let lines = ["First line", "Second line", "Third line", "Fourth line"];
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                buffer.insert_line_break(); // Add newline before each line except first
            }
            for ch in line.chars() {
                buffer.insert_char(ch);
            }
        }
        
        // Position cursor at start of second line
        buffer.cursor = Position::new(1, 0);
        
        // Start visual selection
        buffer.start_visual_selection();
        assert!(buffer.has_selection());
        
        // Move cursor down to select multiple lines (Second line + Third line)
        buffer.cursor = Position::new(2, 10); // End of "Third line"
        buffer.update_visual_selection(Position::new(2, 10));
        
        // Get selected text - should include both lines
        let selected_text = buffer.get_selected_text().unwrap();
        println!("Selected text: '{}'", selected_text);
        
        // Should contain "Second line\nThird line" 
        assert!(selected_text.contains("Second line"));
        assert!(selected_text.contains("Third line"));
        
        // Yank the selection
        let yanked_text = buffer.yank_selection().unwrap();
        println!("Yanked text: '{}'", yanked_text);
        
        // Verify yanked text contains both lines
        assert_eq!(yanked_text, selected_text);
        assert!(yanked_text.contains("Second line"));
        assert!(yanked_text.contains("Third line"));
        
        // Verify clipboard contains the multi-line text
        println!("Clipboard text: '{}'", buffer.clipboard.text);
        assert_eq!(buffer.clipboard.text, yanked_text);
        assert!(buffer.clipboard.text.contains("Second line"));
        assert!(buffer.clipboard.text.contains("Third line"));
        
        // Move cursor to end of fourth line for paste test
        buffer.cursor = Position::new(3, 11); // End of "Fourth line"
        
        // Test the actual paste operation
        let lines_before_paste = buffer.lines.len();
        println!("Lines before paste: {}", lines_before_paste);
        println!("Line 3 before paste: '{}'", buffer.lines[3]);
        
        // Paste the multi-line text
        buffer.put_after();
        
        // Check what happened after paste
        println!("Lines after paste: {}", buffer.lines.len());
        println!("Line 3 after paste: '{}'", buffer.lines[3]);
        if buffer.lines.len() > 4 {
            println!("Line 4 after paste: '{}'", buffer.lines[4]);
        }
        if buffer.lines.len() > 5 {
            println!("Line 5 after paste: '{}'", buffer.lines[5]);
        }
        
        // THE BUG: Multi-line character selection should paste properly
        // The pasted text should include both "Second line" and "Third line"
        let mut found_second_line = false;
        let mut found_third_line = false;
        
        for line in &buffer.lines {
            if line.contains("Second line") {
                found_second_line = true;
            }
            if line.contains("Third line") {
                found_third_line = true;
            }
        }
        
        assert!(found_second_line, "Paste should include 'Second line'");
        assert!(found_third_line, "Paste should include 'Third line'");
    }

    /// Test visual selection range calculation for multi-line selections
    #[test] 
    fn test_multiline_selection_range() {
        let mut buffer = Buffer::new(1, 100);
        
        // Insert test lines
        let lines = ["Line A", "Line B", "Line C"];
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                buffer.insert_line_break();
            }
            for ch in line.chars() {
                buffer.insert_char(ch);
            }
        }
        
        // Start selection at beginning of Line B
        buffer.cursor = Position::new(1, 0);
        buffer.start_visual_selection();
        
        // Extend selection to middle of Line C
        buffer.cursor = Position::new(2, 4); // "Line" in "Line C"
        buffer.update_visual_selection(Position::new(2, 4));
        
        // Get selection range
        let range = buffer.get_selection_range().unwrap();
        println!("Selection range: {:?}", range);
        
        // Should span from (1,0) to (2,4)
        assert_eq!(range.0.row, 1);
        assert_eq!(range.0.col, 0);
        assert_eq!(range.1.row, 2);
        assert_eq!(range.1.col, 4);
        
        // Get selected text
        let selected = buffer.get_selected_text().unwrap();
        println!("Selected text: '{}'", selected);
        
        // Should contain "Line B\nLine" (partial Line C)
        assert!(selected.contains("Line B"));
        assert!(selected.contains('\n'));
        assert!(selected.ends_with("Line"));
        assert!(!selected.contains(" C")); // Should not include " C" part
    }

    /// Test the specific user-reported bug: v + j + y + p with multi-line selection
    #[test]
    fn test_user_reported_multiline_bug_scenario() {
        let mut buffer = Buffer::new(1, 100);
        
        // Create the scenario: multiple lines of text
        let test_lines = ["Line 1: First line", "Line 2: Second line", "Line 3: Third line", "Line 4: Fourth line"];
        for (i, line) in test_lines.iter().enumerate() {
            if i > 0 {
                buffer.insert_line_break();
            }
            for ch in line.chars() {
                buffer.insert_char(ch);
            }
        }
        
        // Position cursor at start of Line 2
        buffer.cursor = Position::new(1, 0);
        
        // Step 1: Press 'v' to enter visual mode
        buffer.start_visual_selection();
        assert!(buffer.has_selection());
        
        // Step 2: Press 'j' to move down and select multiple lines (Line 2 + Line 3)
        buffer.cursor = Position::new(2, 18); // End of "Line 3: Third line"
        buffer.update_visual_selection(Position::new(2, 18));
        
        // Verify we selected the expected text
        let selected_text = buffer.get_selected_text().unwrap();
        println!("Selected with v+j: '{}'", selected_text);
        assert!(selected_text.contains("Line 2: Second line"));
        assert!(selected_text.contains("Line 3: Third line"));
        assert!(selected_text.contains('\n'), "Multi-line selection should contain newline");
        
        // Step 3: Press 'y' to yank the multi-line selection
        let yanked_text = buffer.yank_selection().unwrap();
        println!("Yanked text: '{}'", yanked_text);
        assert_eq!(yanked_text, selected_text);
        
        // Verify selection is cleared after yank
        assert!(!buffer.has_selection(), "Selection should be cleared after yanking");
        
        // Move cursor to end of the buffer for paste test
        buffer.cursor = Position::new(3, 19); // End of "Line 4: Fourth line"
        
        let lines_before = buffer.lines.len();
        println!("Buffer before paste ({} lines):", lines_before);
        for (i, line) in buffer.lines.iter().enumerate() {
            println!("  [{}]: '{}'", i, line);
        }
        
        // Step 4: Press 'p' to paste after cursor
        buffer.put_after();
        
        let lines_after = buffer.lines.len();
        println!("\nBuffer after paste ({} lines):", lines_after);
        for (i, line) in buffer.lines.iter().enumerate() {
            println!("  [{}]: '{}'", i, line);
        }
        
        // THE FIX: Both lines should now be pasted correctly
        assert!(lines_after > lines_before, "Paste should add new lines");
        
        // Look for both pasted lines in the buffer
        let buffer_text = buffer.lines.join("\n");
        
        // Count occurrences - we should have:
        // 1. Original "Line 2: Second line" 
        // 2. Pasted "Line 2: Second line"
        // 3. Original "Line 3: Third line"
        // 4. Pasted "Line 3: Third line"
        let line2_count = buffer_text.matches("Line 2: Second line").count();
        let line3_count = buffer_text.matches("Line 3: Third line").count();
        
        println!("Line 2 occurrences: {}, Line 3 occurrences: {}", line2_count, line3_count);
        
        assert_eq!(line2_count, 2, "Should have original + pasted 'Line 2: Second line'");
        assert_eq!(line3_count, 2, "Should have original + pasted 'Line 3: Third line'");
        
        // The bug was that only the first line would be pasted
        // With the fix, both lines should be pasted correctly
    }
}
