use oxidized::core::buffer::{Buffer, YankType};
use oxidized::core::mode::{Position, SelectionType};

fn create_test_buffer() -> Buffer {
    let mut buffer = Buffer::new(1, 100);
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
    assert_eq!(selection.selection_type, SelectionType::Block);
}

#[test]
fn test_visual_block_selection_update() {
    let mut buffer = create_test_buffer();
    buffer.cursor = Position { row: 0, col: 2 };
    buffer.start_visual_block_selection();

    // Move cursor to create a block selection
    let end_pos = Position { row: 2, col: 4 };
    buffer.update_visual_selection(end_pos);

    let selection = buffer.selection.as_ref().unwrap();
    assert_eq!(selection.start, Position { row: 0, col: 2 });
    assert_eq!(selection.end, Position { row: 2, col: 4 });
    assert_eq!(selection.selection_type, SelectionType::Block);
}

#[test]
fn test_visual_block_yank() {
    let mut buffer = create_test_buffer();
    buffer.cursor = Position { row: 0, col: 5 };
    buffer.start_visual_block_selection();

    // Create a block selection from (0,5) to (2,7)
    buffer.update_visual_selection(Position { row: 2, col: 7 });

    // Yank the block selection
    let yanked_text = buffer.yank_selection();

    // Check clipboard contents
    let expected_text = "one\ntwo\n   "; // Visual block with padding - fixed expectation
    assert!(yanked_text.is_some());
    assert_eq!(buffer.clipboard.text, expected_text);
    assert_eq!(buffer.clipboard.yank_type, YankType::Block);

    // Check that selection is cleared after yank
    assert!(buffer.selection.is_none());
}

#[test]
fn test_visual_block_get_selected_text() {
    let mut buffer = create_test_buffer();
    buffer.cursor = Position { row: 1, col: 0 };
    buffer.start_visual_block_selection();

    // Select block from (1,0) to (3,3)
    buffer.update_visual_selection(Position { row: 3, col: 3 });

    let selected_text = buffer.get_selected_text();
    let expected = "line\nshor\nanot"; // Block selection - fixed expectation
    assert!(selected_text.is_some());
    assert_eq!(selected_text.unwrap(), expected);
}

#[test]
fn test_visual_block_get_selected_text_with_padding() {
    let mut buffer = create_test_buffer();
    buffer.cursor = Position { row: 0, col: 6 };
    buffer.start_visual_block_selection();

    // Select block that extends beyond line length
    buffer.update_visual_selection(Position { row: 2, col: 7 });

    let selected_text = buffer.get_selected_text();
    // Should pad short lines with spaces to maintain rectangular structure
    let expected = "ne\nwo\n  "; // Corrected expectation
    assert!(selected_text.is_some());
    assert_eq!(selected_text.unwrap(), expected);
}

#[test]
fn test_visual_block_paste_after() {
    let mut buffer = create_test_buffer();

    // Set up clipboard with block text
    buffer.clipboard.text = "AB\nCD\nEF".to_string();
    buffer.clipboard.yank_type = YankType::Block;

    // Position cursor and paste (cursor at col 3, which is on 'e')
    buffer.cursor = Position { row: 1, col: 3 };
    buffer.put_after();

    // Check that block was inserted correctly
    assert_eq!(buffer.lines[1], "lineAB two longer");
    assert_eq!(buffer.lines[2], "shorCDt");
    assert_eq!(buffer.lines[3], "anotEFher line here");
    assert_eq!(buffer.cursor, Position { row: 1, col: 5 });
}

#[test]
fn test_visual_block_paste_before() {
    let mut buffer = create_test_buffer();

    // Set up clipboard with block text
    buffer.clipboard.text = "X\nY\nZ".to_string();
    buffer.clipboard.yank_type = YankType::Block;

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
    buffer.clipboard.yank_type = YankType::Block;

    // Position cursor near end of buffer and paste
    buffer.cursor = Position { row: 3, col: 0 };
    buffer.put_after();

    // Check that buffer was extended with new lines
    assert_eq!(buffer.lines.len(), 9); // Original 4 + 5 new lines = 9 total
    assert_eq!(buffer.lines[4], "1");
    assert_eq!(buffer.lines[5], "2");
    assert_eq!(buffer.lines[6], "3");
    assert_eq!(buffer.lines[7], "4");
    assert_eq!(buffer.lines[8], "5");
}

#[test]
fn test_visual_block_clear_selection() {
    let mut buffer = create_test_buffer();
    buffer.cursor = Position { row: 0, col: 0 };
    buffer.start_visual_block_selection();

    assert!(buffer.selection.is_some());

    buffer.clear_visual_selection();

    assert!(buffer.selection.is_none());
}

#[test]
fn test_visual_block_single_character() {
    let mut buffer = create_test_buffer();
    buffer.cursor = Position { row: 1, col: 5 };
    buffer.start_visual_block_selection();

    // Yank single character block
    let yanked_text = buffer.yank_selection();

    // Should contain just the character under cursor
    assert!(yanked_text.is_some());
    assert_eq!(buffer.clipboard.text, "t");
    assert_eq!(buffer.clipboard.yank_type, YankType::Block);
}

#[test]
fn test_visual_block_empty_lines() {
    let mut buffer = Buffer::new(1, 100);
    buffer.lines = vec!["text".to_string(), "".to_string(), "more".to_string()];
    buffer.cursor = Position { row: 0, col: 1 };
    buffer.start_visual_block_selection();

    // Select block that includes empty line
    buffer.update_visual_selection(Position { row: 2, col: 3 });

    let selected_text = buffer.get_selected_text();
    let expected = "ext\n   \nore"; // Empty line padded with spaces
    assert!(selected_text.is_some());
    assert_eq!(selected_text.unwrap(), expected);
}

#[test]
fn test_visual_block_rectangular_selection() {
    let mut buffer = Buffer::new(1, 100);
    buffer.lines = vec![
        "abcdefgh".to_string(),
        "12345678".to_string(),
        "ABCDEFGH".to_string(),
        "!@#$%^&*".to_string(),
    ];
    buffer.cursor = Position { row: 0, col: 2 };
    buffer.start_visual_block_selection();

    // Create rectangular selection from (0,2) to (3,5)
    buffer.update_visual_selection(Position { row: 3, col: 5 });

    let selected_text = buffer.get_selected_text();
    let expected = "cdef\n3456\nCDEF\n#$%^"; // 4x4 rectangular block
    assert!(selected_text.is_some());
    assert_eq!(selected_text.unwrap(), expected);
}

#[test]
fn test_visual_block_paste_rectangular() {
    let mut buffer = Buffer::new(1, 100);
    buffer.lines = vec![
        "aaaa".to_string(),
        "bbbb".to_string(),
        "cccc".to_string(),
        "dddd".to_string(),
    ];

    // Set up rectangular clipboard content
    buffer.clipboard.text = "XX\nYY\nZZ".to_string();
    buffer.clipboard.yank_type = YankType::Block;

    // Position cursor and paste
    buffer.cursor = Position { row: 1, col: 1 };
    buffer.put_after();

    // Check that rectangular block was inserted correctly
    assert_eq!(buffer.lines[1], "bbXXbb");
    assert_eq!(buffer.lines[2], "ccYYcc");
    assert_eq!(buffer.lines[3], "ddZZdd");
    assert_eq!(buffer.cursor, Position { row: 1, col: 3 });
}

#[test]
fn test_visual_block_yank_paste_roundtrip() {
    let mut buffer = create_test_buffer();
    buffer.cursor = Position { row: 0, col: 2 };
    buffer.start_visual_block_selection();

    // Select a 3x2 block
    buffer.update_visual_selection(Position { row: 2, col: 4 });

    // Yank the block
    let yanked_text = buffer.yank_selection();
    assert!(yanked_text.is_some());

    // Move to a different position
    buffer.cursor = Position { row: 3, col: 10 };

    // Paste the block
    buffer.put_after();

    // Verify the block was pasted correctly
    // Original block from (0,2) to (2,4): "ne ", "ne", "ort"
    // Pasted starting at line 4
    assert!(buffer.lines[4].contains("ne"));
    assert!(buffer.lines[5].contains("ne"));
    assert!(buffer.lines[6].contains("ort"));
}
