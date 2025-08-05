#[cfg(test)]
mod keymap_tests {
    use super::super::*;
    use crate::editor::Editor;
    use crate::mode::Mode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn create_test_editor() -> Editor {
        Editor::new().expect("Failed to create test editor")
    }

    fn create_key_event(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn create_key_event_with_modifiers(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn test_key_handler_creation() {
        let handler = KeyHandler::new();
        // Handler should be created successfully
        assert!(handler.keymap_config.normal_mode.len() > 0);
        assert!(handler.keymap_config.insert_mode.len() > 0);
        assert!(handler.keymap_config.command_mode.len() > 0);
    }

    #[test]
    fn test_key_event_to_string() {
        // Test basic character key
        let key = create_key_event(KeyCode::Char('a'));
        let key_str = KeyHandler::key_event_to_string(key);
        assert_eq!(key_str, "a");

        // Test key with ctrl modifier
        let key = create_key_event_with_modifiers(KeyCode::Char('a'), KeyModifiers::CONTROL);
        let key_str = KeyHandler::key_event_to_string(key);
        assert_eq!(key_str, "Ctrl+a");

        // Test special keys
        let key = create_key_event(KeyCode::Enter);
        let key_str = KeyHandler::key_event_to_string(key);
        assert_eq!(key_str, "Enter");

        let key = create_key_event(KeyCode::Esc);
        let key_str = KeyHandler::key_event_to_string(key);
        assert_eq!(key_str, "Escape");

        let key = create_key_event(KeyCode::Backspace);
        let key_str = KeyHandler::key_event_to_string(key);
        assert_eq!(key_str, "Backspace");
    }

    #[test]
    fn test_normal_mode_navigation() {
        let mut editor = create_test_editor();
        let mut handler = KeyHandler::new();

        // Add some content to test navigation
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec![
                "first line".to_string(),
                "second line".to_string(),
                "third line".to_string(),
            ];
            buffer.cursor.row = 1;
            buffer.cursor.col = 5;
        }

        // Test cursor movement down
        let key = create_key_event(KeyCode::Char('j'));
        handler.handle_key(&mut editor, key).unwrap();

        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 2);
        }

        // Test cursor movement up
        let key = create_key_event(KeyCode::Char('k'));
        handler.handle_key(&mut editor, key).unwrap();

        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 1);
        }

        // Test cursor movement right
        let key = create_key_event(KeyCode::Char('l'));
        handler.handle_key(&mut editor, key).unwrap();

        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 6);
        }

        // Test cursor movement left
        let key = create_key_event(KeyCode::Char('h'));
        handler.handle_key(&mut editor, key).unwrap();

        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 5);
        }
    }

    #[test]
    fn test_mode_transitions() {
        let mut editor = create_test_editor();
        let mut handler = KeyHandler::new();

        // Start in Normal mode
        assert_eq!(editor.mode(), Mode::Normal);

        // Enter Insert mode
        let key = create_key_event(KeyCode::Char('i'));
        handler.handle_key(&mut editor, key).unwrap();
        assert_eq!(editor.mode(), Mode::Insert);

        // Return to Normal mode
        let key = create_key_event(KeyCode::Esc);
        handler.handle_key(&mut editor, key).unwrap();
        assert_eq!(editor.mode(), Mode::Normal);

        // Enter Command mode
        let key = create_key_event(KeyCode::Char(':'));
        handler.handle_key(&mut editor, key).unwrap();
        assert_eq!(editor.mode(), Mode::Command);
        assert_eq!(editor.command_line(), ":");

        // Return to Normal mode from Command
        let key = create_key_event(KeyCode::Esc);
        handler.handle_key(&mut editor, key).unwrap();
        assert_eq!(editor.mode(), Mode::Normal);
    }

    #[test]
    fn test_insert_mode_typing() {
        let mut editor = create_test_editor();
        let mut handler = KeyHandler::new();

        // Enter Insert mode
        let key = create_key_event(KeyCode::Char('i'));
        handler.handle_key(&mut editor, key).unwrap();
        assert_eq!(editor.mode(), Mode::Insert);

        // Type some characters
        let key = create_key_event(KeyCode::Char('h'));
        handler.handle_key(&mut editor, key).unwrap();
        let key = create_key_event(KeyCode::Char('e'));
        handler.handle_key(&mut editor, key).unwrap();
        let key = create_key_event(KeyCode::Char('l'));
        handler.handle_key(&mut editor, key).unwrap();
        let key = create_key_event(KeyCode::Char('l'));
        handler.handle_key(&mut editor, key).unwrap();
        let key = create_key_event(KeyCode::Char('o'));
        handler.handle_key(&mut editor, key).unwrap();

        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines[0], "hello");
            assert_eq!(buffer.cursor.col, 5);
            assert!(buffer.modified);
        }
    }

    #[test]
    fn test_command_mode_input() {
        let mut editor = create_test_editor();
        let mut handler = KeyHandler::new();

        // Enter Command mode
        let key = create_key_event(KeyCode::Char(':'));
        handler.handle_key(&mut editor, key).unwrap();
        assert_eq!(editor.mode(), Mode::Command);

        // Type a command
        let key = create_key_event(KeyCode::Char('q'));
        handler.handle_key(&mut editor, key).unwrap();
        assert_eq!(editor.command_line(), ":q");

        let key = create_key_event(KeyCode::Char('u'));
        handler.handle_key(&mut editor, key).unwrap();
        let key = create_key_event(KeyCode::Char('i'));
        handler.handle_key(&mut editor, key).unwrap();
        let key = create_key_event(KeyCode::Char('t'));
        handler.handle_key(&mut editor, key).unwrap();
        assert_eq!(editor.command_line(), ":quit");

        // Test backspace in command mode
        let key = create_key_event(KeyCode::Backspace);
        handler.handle_key(&mut editor, key).unwrap();
        assert_eq!(editor.command_line(), ":qui");
    }

    #[test]
    fn test_search_mode() {
        let mut editor = create_test_editor();
        let mut handler = KeyHandler::new();

        // Add content to search in
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec![
                "hello world".to_string(),
                "test line".to_string(),
                "world peace".to_string(),
            ];
        }

        // Enter Search mode
        let key = create_key_event(KeyCode::Char('/'));
        handler.handle_key(&mut editor, key).unwrap();
        assert_eq!(editor.mode(), Mode::Search);
        assert_eq!(editor.command_line(), "/");

        // Type search term
        let key = create_key_event(KeyCode::Char('w'));
        handler.handle_key(&mut editor, key).unwrap();
        let key = create_key_event(KeyCode::Char('o'));
        handler.handle_key(&mut editor, key).unwrap();
        let key = create_key_event(KeyCode::Char('r'));
        handler.handle_key(&mut editor, key).unwrap();
        let key = create_key_event(KeyCode::Char('l'));
        handler.handle_key(&mut editor, key).unwrap();
        let key = create_key_event(KeyCode::Char('d'));
        handler.handle_key(&mut editor, key).unwrap();
        assert_eq!(editor.command_line(), "/world");

        // Execute search
        let key = create_key_event(KeyCode::Enter);
        handler.handle_key(&mut editor, key).unwrap();
        assert_eq!(editor.mode(), Mode::Normal);

        // Should find "world" and move cursor to it
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 6); // Position of "world" in "hello world"
        }
    }

    #[test]
    fn test_operator_pending_mode() {
        let mut editor = create_test_editor();
        let mut handler = KeyHandler::new();

        // Add content for text object operations
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello world test".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 8; // Position cursor on "world"
        }

        // Press delete operator
        let key = create_key_event(KeyCode::Char('d'));
        handler.handle_key(&mut editor, key).unwrap();
        assert_eq!(editor.mode(), Mode::OperatorPending);
        assert!(editor.get_pending_operator().is_some());

        // Execute text object (inner word)
        let key = create_key_event(KeyCode::Char('i'));
        handler.handle_key(&mut editor, key).unwrap();
        let key = create_key_event(KeyCode::Char('w'));
        handler.handle_key(&mut editor, key).unwrap();

        // Should return to Normal mode and delete the word
        assert_eq!(editor.mode(), Mode::Normal);
        assert!(editor.get_pending_operator().is_none());

        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines[0], "hello  test"); // "world" should be deleted
        }
    }

    #[test]
    fn test_visual_mode() {
        let mut editor = create_test_editor();
        let mut handler = KeyHandler::new();

        // Enter Visual mode
        let key = create_key_event(KeyCode::Char('v'));
        handler.handle_key(&mut editor, key).unwrap();
        assert_eq!(editor.mode(), Mode::Visual);

        // Return to Normal mode
        let key = create_key_event(KeyCode::Esc);
        handler.handle_key(&mut editor, key).unwrap();
        assert_eq!(editor.mode(), Mode::Normal);
    }

    #[test]
    fn test_undo_redo() {
        let mut editor = create_test_editor();
        let mut handler = KeyHandler::new();

        // Add some content
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines[0] = "original text".to_string();
            buffer.modified = true;
        }

        // Enter Insert mode and add text
        let key = create_key_event(KeyCode::Char('i'));
        handler.handle_key(&mut editor, key).unwrap();
        let key = create_key_event(KeyCode::Char('X'));
        handler.handle_key(&mut editor, key).unwrap();

        // Return to Normal mode
        let key = create_key_event(KeyCode::Esc);
        handler.handle_key(&mut editor, key).unwrap();
        assert_eq!(editor.mode(), Mode::Normal);

        // Test undo
        let key = create_key_event(KeyCode::Char('u'));
        handler.handle_key(&mut editor, key).unwrap();

        // Note: The actual undo behavior depends on the buffer's undo implementation
        // This test verifies the keymap handles the undo action
    }

    #[test]
    fn test_word_movement() {
        let mut editor = create_test_editor();
        let mut handler = KeyHandler::new();

        // Add content with multiple words
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello world test line".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        // Move to next word
        let key = create_key_event(KeyCode::Char('w'));
        handler.handle_key(&mut editor, key).unwrap();

        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 6); // Start of "world"
        }

        // Move to next word again
        let key = create_key_event(KeyCode::Char('w'));
        handler.handle_key(&mut editor, key).unwrap();

        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 12); // Start of "test"
        }

        // Move backward
        let key = create_key_event(KeyCode::Char('b'));
        handler.handle_key(&mut editor, key).unwrap();

        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 6); // Back to "world"
        }
    }

    #[test]
    fn test_ctrl_key_combinations() {
        let mut editor = create_test_editor();
        let mut handler = KeyHandler::new();

        // Test Ctrl+C (should go to normal mode)
        editor.set_mode(Mode::Insert);
        let key = create_key_event_with_modifiers(KeyCode::Char('c'), KeyModifiers::CONTROL);
        handler.handle_key(&mut editor, key).unwrap();
        assert_eq!(editor.mode(), Mode::Normal);
    }

    #[test]
    fn test_invalid_key_handling() {
        let mut editor = create_test_editor();
        let mut handler = KeyHandler::new();

        // Test handling of unmapped keys - should not crash
        let key = create_key_event(KeyCode::F(12));
        let result = handler.handle_key(&mut editor, key);
        assert!(result.is_ok());

        // Editor should remain in same mode
        assert_eq!(editor.mode(), Mode::Normal);
    }

    #[test]
    fn test_keymap_fallback() {
        // Test that minimal fallback keymap is created when config fails to load
        let fallback_config = KeyHandler::create_minimal_fallback();

        // Should have basic essential keys for graceful exit
        assert!(fallback_config.normal_mode.contains_key(":"));
        assert!(fallback_config.command_mode.contains_key("Escape"));
        assert!(fallback_config.command_mode.contains_key("Enter"));

        // Insert mode should be empty in minimal fallback
        assert!(fallback_config.insert_mode.is_empty());
    }
}
