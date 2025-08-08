use oxidized::core::buffer::Buffer;
use oxidized::core::mode::Position;

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
    buffer.cursor.col = 1;
    buffer.delete_char();

    // delete_char appears to delete the character before the cursor
    assert_eq!(buffer.lines[0], "ello");
    assert!(buffer.modified);
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
fn test_cursor_movement() {
    let mut buffer = create_buffer_with_content("line1\nline2\nline3");

    // Test cursor positioning
    let new_pos = Position { row: 1, col: 2 };
    buffer.move_cursor(new_pos);
    assert_eq!(buffer.cursor.row, 1);
    assert_eq!(buffer.cursor.col, 2);
}

#[test]
fn test_word_operations() {
    let mut buffer = create_buffer_with_content("hello world test");
    buffer.cursor.col = 0;

    // Test moving to next word
    buffer.move_to_next_word();
    // Should be at start of "world"
    assert!(buffer.cursor.col > 0);

    // Test moving to previous word
    buffer.move_to_previous_word();
    // Should be back at start
    assert_eq!(buffer.cursor.col, 0);

    // Test yanking word
    buffer.yank_word();
    // Test that put after works
    buffer.put_after();
}

#[test]
fn test_undo_redo() {
    let mut buffer = create_test_buffer();

    // Perform an edit
    buffer.insert_char('a');
    assert_eq!(buffer.lines[0], "a");

    // Undo
    buffer.undo();
    assert_eq!(buffer.lines[0], "");

    // Redo
    buffer.redo();
    assert_eq!(buffer.lines[0], "a");
}

#[test]
fn test_line_operations() {
    let mut buffer = create_buffer_with_content("line1\nline2\nline3");
    buffer.cursor.row = 1; // Second line

    // Delete line
    buffer.delete_line();
    assert_eq!(buffer.lines.len(), 2);
    assert_eq!(buffer.lines[0], "line1");
    assert_eq!(buffer.lines[1], "line3");
}

#[test]
fn test_multiple_edits() {
    let mut buffer = create_test_buffer();

    // Insert some text
    buffer.insert_char('H');
    buffer.insert_char('e');
    buffer.insert_char('l');
    buffer.insert_char('l');
    buffer.insert_char('o');

    assert_eq!(buffer.lines[0], "Hello");
    assert!(buffer.modified);

    // Add a space and more text
    buffer.insert_char(' ');
    buffer.insert_char('W');
    buffer.insert_char('o');
    buffer.insert_char('r');
    buffer.insert_char('l');
    buffer.insert_char('d');

    assert_eq!(buffer.lines[0], "Hello World");
}

#[test]
fn test_buffer_api() {
    let buffer = create_buffer_with_content("line1\nline2\nline3");

    // Test line count
    assert_eq!(buffer.line_count(), 3);

    // Test getting lines
    assert_eq!(buffer.get_line(0), Some(&"line1".to_string()));
    assert_eq!(buffer.get_line(1), Some(&"line2".to_string()));
    assert_eq!(buffer.get_line(2), Some(&"line3".to_string()));
    assert_eq!(buffer.get_line(3), None);
}

#[test]
fn test_delete_operations() {
    let mut buffer = create_buffer_with_content("hello world");

    // Test delete char at cursor
    buffer.cursor.col = 5; // At space
    buffer.delete_char_at_cursor();
    assert_eq!(buffer.lines[0], "helloworld");

    // Test delete char before cursor
    buffer.cursor.col = 5; // At 'w'
    buffer.delete_char_before_cursor();
    assert_eq!(buffer.lines[0], "hellworld");
}

#[test]
fn test_empty_buffer_operations() {
    let mut buffer = create_test_buffer();

    // Test operations on empty buffer
    buffer.delete_char(); // Should not panic

    // Buffer should still be valid
    assert_eq!(buffer.lines.len(), 1);
    assert_eq!(buffer.lines[0], "");
}
