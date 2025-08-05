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

    // Character navigation tests
    #[test]
    fn test_find_char_forward() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Ensure we have a buffer
        editor.create_buffer(None).expect("Failed to create buffer");

        // Set up test content directly
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello world".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        // Test finding 'l' forward from position 0
        let key = create_key_event(KeyCode::Char('l'));
        let result = handler.action_find_char_forward(&mut editor, key);
        assert!(result.is_ok());

        // Should move to first 'l' at position 2
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 2);
        }

        // Test that search state is stored
        assert!(handler.last_char_search.is_some());
        let search_state = handler.last_char_search.as_ref().unwrap();
        assert_eq!(search_state.character, 'l');
        assert_eq!(search_state.search_type, CharSearchType::Find);
        assert!(search_state.forward);
    }

    #[test]
    fn test_find_char_backward() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Ensure we have a buffer
        editor.create_buffer(None).expect("Failed to create buffer");

        // Set up test content directly
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello world".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 10;
        }

        // Test finding 'l' backward from end
        let key = create_key_event(KeyCode::Char('l'));
        let result = handler.action_find_char_backward(&mut editor, key);
        assert!(result.is_ok());

        // Should move to last 'l' at position 9
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 9);
        }

        // Test that search state is stored
        assert!(handler.last_char_search.is_some());
        let search_state = handler.last_char_search.as_ref().unwrap();
        assert_eq!(search_state.character, 'l');
        assert_eq!(search_state.search_type, CharSearchType::Find);
        assert!(!search_state.forward);
    }

    #[test]
    fn test_till_char_forward() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Ensure we have a buffer
        editor.create_buffer(None).expect("Failed to create buffer");

        // Set up test content directly
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello world".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        // Test till 'o' forward from position 0
        let key = create_key_event(KeyCode::Char('o'));
        let result = handler.action_till_char_forward(&mut editor, key);
        assert!(result.is_ok());

        // Should move to position before 'o' at position 3 (till stops before character)
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 3);
        }

        // Test that search state is stored
        assert!(handler.last_char_search.is_some());
        let search_state = handler.last_char_search.as_ref().unwrap();
        assert_eq!(search_state.character, 'o');
        assert_eq!(search_state.search_type, CharSearchType::Till);
        assert!(search_state.forward);
    }

    #[test]
    fn test_till_char_backward() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Ensure we have a buffer
        editor.create_buffer(None).expect("Failed to create buffer");

        // Set up test content directly
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello world".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 10;
        }

        // Test till 'o' backward from end
        let key = create_key_event(KeyCode::Char('o'));
        let result = handler.action_till_char_backward(&mut editor, key);
        assert!(result.is_ok());

        // Should move to position after 'o' at position 8 (till stops after character)
        // Note: "hello world" has 'o' at positions 4 and 7, rfind finds the last one at 7
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 8);
        }

        // Test that search state is stored
        assert!(handler.last_char_search.is_some());
        let search_state = handler.last_char_search.as_ref().unwrap();
        assert_eq!(search_state.character, 'o');
        assert_eq!(search_state.search_type, CharSearchType::Till);
        assert!(!search_state.forward);
    }

    #[test]
    fn test_repeat_char_search() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Ensure we have a buffer
        editor.create_buffer(None).expect("Failed to create buffer");

        // Set up test content directly
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello hello".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        // First, do a find forward for 'l'
        let key = create_key_event(KeyCode::Char('l'));
        let result = handler.action_find_char_forward(&mut editor, key);
        assert!(result.is_ok());

        // Should be at first 'l' (position 2)
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 2);
        }

        // Now repeat the search
        let result = handler.action_repeat_char_search(&mut editor);
        assert!(result.is_ok());

        // Should move to second 'l' (position 3)
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 3);
        }

        // Repeat again
        let result = handler.action_repeat_char_search(&mut editor);
        assert!(result.is_ok());

        // Should move to third 'l' (position 8)
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 8);
        }
    }

    #[test]
    fn test_repeat_char_search_reverse() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Ensure we have a buffer
        editor.create_buffer(None).expect("Failed to create buffer");

        // Set up test content directly
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello hello".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 5;
        }

        // First, do a find forward for 'l'
        let key = create_key_event(KeyCode::Char('l'));
        let result = handler.action_find_char_forward(&mut editor, key);
        assert!(result.is_ok());

        // Should be at position 8 (first 'l' in second "hello")
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 8);
        }

        // Now repeat the search in reverse
        let result = handler.action_repeat_char_search_reverse(&mut editor);
        assert!(result.is_ok());

        // Should move backward to position 3 (second 'l' in first "hello")
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 3);
        }
    }

    #[test]
    fn test_character_not_found() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Ensure we have a buffer
        editor.create_buffer(None).expect("Failed to create buffer");

        // Set up test content directly
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello world".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        let original_col = if let Some(buffer) = editor.current_buffer() {
            buffer.cursor.col
        } else {
            0
        };

        // Test finding a character that doesn't exist
        let key = create_key_event(KeyCode::Char('z'));
        let result = handler.action_find_char_forward(&mut editor, key);
        assert!(result.is_ok());

        // Cursor should not move
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, original_col);
        }
    }

    #[test]
    fn test_repeat_without_previous_search() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Try to repeat without any previous search
        let result = handler.action_repeat_char_search(&mut editor);
        assert!(result.is_ok()); // Should not error, just do nothing

        let result = handler.action_repeat_char_search_reverse(&mut editor);
        assert!(result.is_ok()); // Should not error, just do nothing
    }

    #[test]
    fn test_char_search_state_storage() {
        let mut handler = KeyHandler::new();

        // Initially no search state
        assert!(handler.last_char_search.is_none());

        // Create search state
        handler.last_char_search = Some(CharSearchState {
            search_type: CharSearchType::Find,
            character: 'x',
            forward: true,
        });

        // Verify state is stored correctly
        let search_state = handler.last_char_search.as_ref().unwrap();
        assert_eq!(search_state.character, 'x');
        assert_eq!(search_state.search_type, CharSearchType::Find);
        assert!(search_state.forward);
    }

    #[test]
    fn test_character_navigation_full_flow() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Ensure we have a buffer
        editor.create_buffer(None).expect("Failed to create buffer");

        // Set up test content
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello world".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        // Test full find character forward flow
        // First press 'f' to start the command
        let f_key = create_key_event(KeyCode::Char('f'));
        let result = handler.handle_key(&mut editor, f_key);
        assert!(result.is_ok());

        // Should be in pending state
        assert!(handler.pending_char_command.is_some());

        // Then press 'l' to find the letter
        let l_key = create_key_event(KeyCode::Char('l'));
        let result = handler.handle_key(&mut editor, l_key);
        assert!(result.is_ok());

        // Should no longer be in pending state
        assert!(handler.pending_char_command.is_none());

        // Should move to first 'l' at position 2
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 2);
        }

        // Should have stored search state
        assert!(handler.last_char_search.is_some());
        let search_state = handler.last_char_search.as_ref().unwrap();
        assert_eq!(search_state.character, 'l');
        assert_eq!(search_state.search_type, CharSearchType::Find);
        assert!(search_state.forward);
    }

    #[test]
    fn test_character_navigation_cancel_on_non_char() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Ensure we have a buffer
        editor.create_buffer(None).expect("Failed to create buffer");

        // Set up test content
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello world".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        // Start find character forward
        let f_key = create_key_event(KeyCode::Char('f'));
        let result = handler.handle_key(&mut editor, f_key);
        assert!(result.is_ok());

        // Should be in pending state
        assert!(handler.pending_char_command.is_some());

        // Press Escape (non-character key)
        let esc_key = create_key_event(KeyCode::Esc);
        let result = handler.handle_key(&mut editor, esc_key);
        assert!(result.is_ok());

        // Should no longer be in pending state (command canceled)
        assert!(handler.pending_char_command.is_none());

        // Cursor should not have moved
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 0);
        }
    }

    #[test]
    fn test_character_navigation_find_backward() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Ensure we have a buffer
        editor.create_buffer(None).expect("Failed to create buffer");

        // Set up test content and position cursor at end
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello world".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 10; // Position at end of line
        }

        // Test full find character backward flow (uppercase F)
        // First press 'F' to start the command
        let f_key = create_key_event_with_modifiers(KeyCode::Char('f'), KeyModifiers::SHIFT);
        let result = handler.handle_key(&mut editor, f_key);
        assert!(result.is_ok());

        // Should be in pending state
        assert!(handler.pending_char_command.is_some());
        let pending = handler.pending_char_command.unwrap();
        assert_eq!(pending.search_type, CharSearchType::Find);
        assert!(!pending.forward); // Should be backward

        // Then press 'l' to find the letter backward
        let l_key = create_key_event(KeyCode::Char('l'));
        let result = handler.handle_key(&mut editor, l_key);
        assert!(result.is_ok());

        // Should no longer be in pending state
        assert!(handler.pending_char_command.is_none());

        // Should move to last 'l' at position 9 (searching backward from position 10)
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 9);
        }

        // Should have stored search state
        assert!(handler.last_char_search.is_some());
        let search_state = handler.last_char_search.as_ref().unwrap();
        assert_eq!(search_state.character, 'l');
        assert_eq!(search_state.search_type, CharSearchType::Find);
        assert!(!search_state.forward); // Should be backward
    }

    #[test]
    fn test_till_char_repeat_functionality() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with test content
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello there everyone".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 3; // Start at second 'l' in "hello"
        }

        // Test 't' (till) command followed by ';' (repeat)
        // First press 't' to start till command
        let t_key = create_key_event(KeyCode::Char('t'));
        let result = handler.handle_key(&mut editor, t_key);
        assert!(result.is_ok());
        assert!(handler.pending_char_command.is_some());

        // Then press 'e' to search till first 'e' (should stop at position 1, before 'e' at position 2)
        let e_key = create_key_event(KeyCode::Char('e'));
        let result = handler.handle_key(&mut editor, e_key);
        assert!(result.is_ok());
        assert!(handler.pending_char_command.is_none());

        // Should be at position 7 (just before the 'e' in "there" at position 8)
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 7);
        }

        // Store search state for verification
        assert!(handler.last_char_search.is_some());
        let search_state = handler.last_char_search.as_ref().unwrap();
        assert_eq!(search_state.search_type, CharSearchType::Till);
        assert_eq!(search_state.character, 'e');
        assert!(search_state.forward);

        // Now press ';' to repeat the search - should go to next 'e'
        let semicolon_key = create_key_event(KeyCode::Char(';'));
        let result = handler.handle_key(&mut editor, semicolon_key);
        assert!(result.is_ok());

        // Should now be at position 9 (just before the 'e' in "there" at position 10)
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 9); // Should move to position before next 'e'
        }
    }

    #[test]
    fn test_reverse_repeat_functionality() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with test content that has multiple 'e' characters
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["test example text".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 16; // Start at end of line
        }

        // Debug: print the line content with positions
        if let Some(buffer) = editor.current_buffer() {
            let line = &buffer.lines[0];
            println!("Line content: '{}'", line);
            for (i, ch) in line.chars().enumerate() {
                if ch == 'e' {
                    println!("'e' found at position {}", i);
                }
            }
        }

        // First do a backward find for 'e' (F command)
        let f_key = create_key_event(KeyCode::Char('F'));
        let result = handler.handle_key(&mut editor, f_key);
        assert!(result.is_ok());
        assert!(handler.pending_char_command.is_some());

        // Then press 'e' to search backward for 'e'
        let e_key = create_key_event(KeyCode::Char('e'));
        let result = handler.handle_key(&mut editor, e_key);
        assert!(result.is_ok());
        assert!(handler.pending_char_command.is_none());

        // Check where we ended up after backward search
        if let Some(buffer) = editor.current_buffer() {
            println!(
                "After backward search, cursor at position: {}",
                buffer.cursor.col
            );
            let expected_pos = 14; // The 'e' in "t[e]xt" should be at position 14
            assert_eq!(buffer.cursor.col, expected_pos);
        }

        // Verify search state
        assert!(handler.last_char_search.is_some());
        let search_state = handler.last_char_search.as_ref().unwrap();
        assert_eq!(search_state.search_type, CharSearchType::Find);
        assert_eq!(search_state.character, 'e');
        assert!(!search_state.forward); // backward search

        // Now press ',' to reverse repeat (should go forward and find next 'e')
        let comma_key = create_key_event(KeyCode::Char(','));
        let result = handler.handle_key(&mut editor, comma_key);
        assert!(result.is_ok());

        // Check where we ended up after reverse repeat
        if let Some(buffer) = editor.current_buffer() {
            println!(
                "After reverse repeat, cursor at position: {}",
                buffer.cursor.col
            );
            // Since we started from position 14 and searched forward,
            // there are no more 'e' characters after position 14 in "test example text"
            // So it should stay at position 14
            assert_eq!(buffer.cursor.col, 14);
        }
    }

    #[test]
    fn test_till_char_backward_repeat_functionality() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with test content that has multiple 'e' characters
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["test example everyone".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 20; // Start at end of line
        }

        // Debug: print the line content with positions
        if let Some(buffer) = editor.current_buffer() {
            let line = &buffer.lines[0];
            println!("Line content: '{}'", line);
            println!("Line length: {}", line.len());
            for (i, ch) in line.chars().enumerate() {
                if ch == 'e' {
                    println!("'e' found at position {}", i);
                }
            }
        }

        // First do a backward till for 'e' (T command)
        let t_key = create_key_event(KeyCode::Char('T'));
        let result = handler.handle_key(&mut editor, t_key);
        assert!(result.is_ok());
        assert!(handler.pending_char_command.is_some());

        // Then press 'e' to search till 'e' backward
        let e_key = create_key_event(KeyCode::Char('e'));
        let result = handler.handle_key(&mut editor, e_key);
        assert!(result.is_ok());
        assert!(handler.pending_char_command.is_none());

        // Check where we ended up after first till backward
        if let Some(buffer) = editor.current_buffer() {
            println!("After first 'Te': cursor at {}", buffer.cursor.col);
            // Starting from position 20, backward till 'e' should find the 'e' at position 15
            // and place us at position 16 (after the 'e')
            assert_eq!(buffer.cursor.col, 16);
        }

        // Verify search state
        assert!(handler.last_char_search.is_some());
        let search_state = handler.last_char_search.as_ref().unwrap();
        assert_eq!(search_state.search_type, CharSearchType::Till);
        assert_eq!(search_state.character, 'e');
        assert!(!search_state.forward); // backward search

        // Now press ';' to repeat the search - should go to next 'e' backward
        let semicolon_key = create_key_event(KeyCode::Char(';'));
        let result = handler.handle_key(&mut editor, semicolon_key);
        assert!(result.is_ok());

        // Should move to after the previous 'e'
        if let Some(buffer) = editor.current_buffer() {
            println!("After repeat (';'): cursor at {}", buffer.cursor.col);
            // The next 'e' backward should be at position 13, so till should place us at position 14
            assert_eq!(buffer.cursor.col, 14);
        }
    }

    #[test]
    fn test_find_char_backward_repeat_functionality() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with test content that has multiple 'e' characters
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["test example everyone".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 20; // Start at end of line
        }

        // First do a backward find for 'e' (F command)
        let f_key = create_key_event(KeyCode::Char('F'));
        let result = handler.handle_key(&mut editor, f_key);
        assert!(result.is_ok());
        assert!(handler.pending_char_command.is_some());

        // Then press 'e' to search find 'e' backward
        let e_key = create_key_event(KeyCode::Char('e'));
        let result = handler.handle_key(&mut editor, e_key);
        assert!(result.is_ok());
        assert!(handler.pending_char_command.is_none());

        // Check where we ended up after first find backward
        if let Some(buffer) = editor.current_buffer() {
            println!("After first 'Fe': cursor at {}", buffer.cursor.col);
            // Starting from position 20, backward find 'e' should find the nearest 'e' backward
            // which is at position 15
            assert_eq!(buffer.cursor.col, 15);
        }

        // Verify search state
        assert!(handler.last_char_search.is_some());
        let search_state = handler.last_char_search.as_ref().unwrap();
        assert_eq!(search_state.search_type, CharSearchType::Find);
        assert_eq!(search_state.character, 'e');
        assert!(!search_state.forward); // backward search

        // Now press ';' to repeat the search - should go to next 'e' backward
        let semicolon_key = create_key_event(KeyCode::Char(';'));
        let result = handler.handle_key(&mut editor, semicolon_key);
        assert!(result.is_ok());

        // Should move to the previous 'e'
        if let Some(buffer) = editor.current_buffer() {
            println!("After repeat (';'): cursor at {}", buffer.cursor.col);
            // The next 'e' backward should be at position 13
            assert_eq!(buffer.cursor.col, 13);
        }
    }

    #[test]
    fn test_comprehensive_till_backward_repeat() {
        // This test demonstrates the fix for the user's issue:
        // "When I press T to search backward, pressing ; doesn't advance to the next match."
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["one two three four".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 17; // Start at end of line
        }

        // Debug: Show character positions
        if let Some(buffer) = editor.current_buffer() {
            let line = &buffer.lines[0];
            println!("Line: '{}'", line);
            println!("Length: {}", line.len());
            for (i, ch) in line.chars().enumerate() {
                if ch == 'o' {
                    println!("'o' found at position {}", i);
                }
            }
        }

        // Press 'T' to start till backward search
        let t_key = create_key_event(KeyCode::Char('T'));
        handler.handle_key(&mut editor, t_key).unwrap();

        // Press 'o' to find 'o' till backward
        let o_key = create_key_event(KeyCode::Char('o'));
        handler.handle_key(&mut editor, o_key).unwrap();

        // Check first position
        if let Some(buffer) = editor.current_buffer() {
            println!("After first 'To': cursor at {}", buffer.cursor.col);
            // Based on "one two three four", 'o' positions: 0(one), 6(two), 15(four)
            // Starting from 17, should find 'o' at 15 and place cursor at 16
            assert_eq!(buffer.cursor.col, 16); // After 'o' in "f[o]ur"
        }

        // Press ';' to repeat - should advance to next 'o' backward
        let semicolon_key = create_key_event(KeyCode::Char(';'));
        handler.handle_key(&mut editor, semicolon_key).unwrap();

        // Check second position
        if let Some(buffer) = editor.current_buffer() {
            println!("After first repeat: cursor at {}", buffer.cursor.col);
            // Should advance to position after 'o' in "two" (position 7)
            assert_eq!(buffer.cursor.col, 7); // After 'o' in "tw[o]"
        }

        // Press ';' again - should advance to next 'o' backward
        handler.handle_key(&mut editor, semicolon_key).unwrap();

        // Check third position
        if let Some(buffer) = editor.current_buffer() {
            println!("After second repeat: cursor at {}", buffer.cursor.col);
            // Should advance to position after 'o' in "one" (position 1)
            assert_eq!(buffer.cursor.col, 1); // After 'o' in "[o]ne"
        }
    }

    // Line operations tests
    #[test]
    fn test_delete_to_end_of_line() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with test content
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello world test".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 6; // Position at 'w' in "world"
        }

        // Test delete to end of line (D command)
        let result = handler.action_delete_to_end_of_line(&mut editor);
        assert!(result.is_ok());

        // Check that text from cursor to end was deleted
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines[0], "hello ");
            assert_eq!(buffer.cursor.col, 6); // Cursor should stay at same position
        }
    }

    #[test]
    fn test_delete_to_end_of_line_at_end() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with cursor at end of line
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 5; // At end of line
        }

        let original_line = if let Some(buffer) = editor.current_buffer() {
            buffer.lines[0].clone()
        } else {
            String::new()
        };

        // Test delete to end of line when already at end
        let result = handler.action_delete_to_end_of_line(&mut editor);
        assert!(result.is_ok());

        // Line should remain unchanged
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines[0], original_line);
        }
    }

    #[test]
    fn test_join_lines() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with multiple lines
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec![
                "first line".to_string(),
                "second line".to_string(),
                "third line".to_string(),
            ];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        // Test join lines (J command)
        let result = handler.action_join_lines(&mut editor);
        assert!(result.is_ok());

        // Check that lines were joined
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines.len(), 2);
            assert_eq!(buffer.lines[0], "first line second line");
            assert_eq!(buffer.lines[1], "third line");
        }
    }

    #[test]
    fn test_join_lines_at_last_line() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with cursor at last line
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["only line".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        let original_lines_count = if let Some(buffer) = editor.current_buffer() {
            buffer.lines.len()
        } else {
            0
        };

        // Test join lines when at last line
        let result = handler.action_join_lines(&mut editor);
        assert!(result.is_ok());

        // Lines should remain unchanged
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines.len(), original_lines_count);
            assert_eq!(buffer.lines[0], "only line");
        }
    }

    #[test]
    fn test_join_lines_with_whitespace() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create lines with trailing/leading whitespace
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec![
                "line with spaces   ".to_string(),
                "   indented line".to_string(),
            ];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        // Test join lines with whitespace trimming
        let result = handler.action_join_lines(&mut editor);
        assert!(result.is_ok());

        // Check that whitespace was properly handled
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines.len(), 1);
            assert_eq!(buffer.lines[0], "line with spaces indented line");
        }
    }

    #[test]
    fn test_change_to_end_of_line() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with test content
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello world test".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 6; // Position at 'w' in "world"
        }

        // Test change to end of line (C command)
        let result = handler.action_change_to_end_of_line(&mut editor);
        assert!(result.is_ok());

        // Check that text was deleted and mode changed to insert
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines[0], "hello ");
        }
        assert_eq!(editor.mode(), Mode::Insert);
    }

    #[test]
    fn test_change_entire_line() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with test content
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello world test".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 6; // Position somewhere in the line
        }

        // Test change entire line (S command)
        let result = handler.action_change_entire_line(&mut editor);
        assert!(result.is_ok());

        // Check that entire line was cleared and cursor moved to beginning
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines[0], "");
            assert_eq!(buffer.cursor.col, 0);
        }
        assert_eq!(editor.mode(), Mode::Insert);
    }

    #[test]
    fn test_substitute_char() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with test content
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 1; // Position at 'e'
        }

        // Test substitute character (s command)
        let result = handler.action_substitute_char(&mut editor);
        assert!(result.is_ok());

        // Check that character was deleted and mode changed to insert
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines[0], "hllo"); // 'e' should be deleted
            assert_eq!(buffer.cursor.col, 1); // Cursor should stay at same position
        }
        assert_eq!(editor.mode(), Mode::Insert);
    }

    #[test]
    fn test_join_lines_full_flow() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with multiple lines
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["first".to_string(), "second".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        // Test J key binding through full key handling
        let j_key = create_key_event(KeyCode::Char('J'));
        let result = handler.handle_key(&mut editor, j_key);
        assert!(result.is_ok());

        // Check that join lines worked
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines.len(), 1);
            assert_eq!(buffer.lines[0], "first second");
        }
    }

    // Bracket Matching Tests
    #[test]
    fn test_bracket_match_parentheses() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with parentheses
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["function(arg1, arg2)".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 8; // Position on opening parenthesis
        }

        // Test bracket matching with % key
        let result = handler.action_bracket_match(&mut editor);
        assert!(result.is_ok());

        // Check cursor moved to closing parenthesis
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 19); // Position of closing parenthesis
        }
    }

    #[test]
    fn test_bracket_match_square_brackets() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with square brackets
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["array[index]".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 5; // Position on opening bracket
        }

        let result = handler.action_bracket_match(&mut editor);
        assert!(result.is_ok());

        // Check cursor moved to closing bracket
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 11); // Position of closing bracket
        }
    }

    #[test]
    fn test_bracket_match_curly_braces() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with curly braces
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec![
                "if condition {".to_string(),
                "    code();".to_string(),
                "}".to_string(),
            ];
            buffer.cursor.row = 0;
            buffer.cursor.col = 13; // Position on opening brace
        }

        let result = handler.action_bracket_match(&mut editor);
        assert!(result.is_ok());

        // Check cursor moved to closing brace
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 2);
            assert_eq!(buffer.cursor.col, 0); // Position of closing brace
        }
    }

    #[test]
    fn test_bracket_match_angle_brackets() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with angle brackets
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["<tag>content</tag>".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0; // Position on opening angle bracket
        }

        let result = handler.action_bracket_match(&mut editor);
        assert!(result.is_ok());

        // Check cursor moved to closing angle bracket
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 4); // Position of closing angle bracket
        }
    }

    #[test]
    fn test_bracket_match_closing_to_opening() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with nested parentheses
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["func(nested(inner))".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 18; // Position on outermost closing parenthesis
        }

        let result = handler.action_bracket_match(&mut editor);
        assert!(result.is_ok());

        // Check cursor moved to outermost opening parenthesis
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 4); // Position of outermost opening parenthesis
        }
    }

    #[test]
    fn test_bracket_match_nested_brackets() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with nested brackets
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["outer(inner(deep))".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 11; // Position on inner opening parenthesis
        }

        let result = handler.action_bracket_match(&mut editor);
        assert!(result.is_ok());

        // Check cursor moved to inner closing parenthesis
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 16); // Position of inner closing parenthesis
        }
    }

    #[test]
    fn test_bracket_match_no_match() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with unmatched bracket
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["unmatched(".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 9; // Position on unmatched opening parenthesis
        }

        let result = handler.action_bracket_match(&mut editor);
        assert!(result.is_ok());

        // Check cursor didn't move (no match found)
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 9); // Cursor should stay in same position
        }
    }

    #[test]
    fn test_bracket_match_not_on_bracket() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["some text here".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 5; // Position on 't' in "text"
        }

        let result = handler.action_bracket_match(&mut editor);
        assert!(result.is_ok());

        // Check cursor didn't move (not on a bracket)
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 5); // Cursor should stay in same position
        }
    }

    #[test]
    fn test_bracket_match_multiline() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with multiline brackets
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec![
                "function() {".to_string(),
                "    if (condition) {".to_string(),
                "        code();".to_string(),
                "    }".to_string(),
                "}".to_string(),
            ];
            buffer.cursor.row = 1;
            buffer.cursor.col = 7; // Position on opening parenthesis in if statement
        }

        let result = handler.action_bracket_match(&mut editor);
        assert!(result.is_ok());

        // Check cursor moved to closing parenthesis
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 1);
            assert_eq!(buffer.cursor.col, 17); // Position of closing parenthesis
        }
    }

    #[test]
    fn test_bracket_match_full_flow() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with brackets
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["test(args)".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 4; // Position on opening parenthesis
        }

        // Test % key binding through full key handling
        let percent_key = create_key_event(KeyCode::Char('%'));
        let result = handler.handle_key(&mut editor, percent_key);
        assert!(result.is_ok());

        // Check that bracket matching worked
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 9); // Position of closing parenthesis
        }
    }

    #[test]
    fn test_bracket_match_backward_edge_case() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer where the opening bracket is at position 0
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["(test)".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 5; // Position on closing parenthesis
        }

        let result = handler.action_bracket_match(&mut editor);
        assert!(result.is_ok());

        // Check cursor moved to opening parenthesis at position 0
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 0); // Position of opening parenthesis
        }
    }

    #[test]
    fn test_bracket_match_multiline_backward() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer where opening bracket is at start of line
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["{".to_string(), "    content".to_string(), "}".to_string()];
            buffer.cursor.row = 2;
            buffer.cursor.col = 0; // Position on closing brace
        }

        let result = handler.action_bracket_match(&mut editor);
        assert!(result.is_ok());

        // Check cursor moved to opening brace at position 0 of first line
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 0); // Position of opening brace
        }
    }

    // Paragraph movement tests
    #[test]
    fn test_paragraph_forward_basic() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with two paragraphs separated by an empty line
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec![
                "first paragraph".to_string(),
                "line two".to_string(),
                "".to_string(), // empty line separating paragraphs
                "second paragraph".to_string(),
                "another line".to_string(),
            ];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        let result = handler.action_paragraph_forward(&mut editor);
        assert!(result.is_ok());

        // Should move to start of second paragraph
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 3);
            assert_eq!(buffer.cursor.col, 0);
        }
    }

    #[test]
    fn test_paragraph_forward_multiple_empty_lines() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with multiple empty lines between paragraphs
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec![
                "first paragraph".to_string(),
                "".to_string(),
                "".to_string(), // multiple empty lines
                "".to_string(),
                "second paragraph".to_string(),
            ];
            buffer.cursor.row = 0;
            buffer.cursor.col = 5;
        }

        let result = handler.action_paragraph_forward(&mut editor);
        assert!(result.is_ok());

        // Should move to start of second paragraph
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 4);
            assert_eq!(buffer.cursor.col, 0);
        }
    }

    #[test]
    fn test_paragraph_forward_from_end() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer and start from the last paragraph
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec![
                "first paragraph".to_string(),
                "".to_string(),
                "second paragraph".to_string(),
                "last line".to_string(),
            ];
            buffer.cursor.row = 2; // Start in second paragraph
            buffer.cursor.col = 0;
        }

        let result = handler.action_paragraph_forward(&mut editor);
        assert!(result.is_ok());

        // Should move to last line (end of file)
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 3);
            assert_eq!(buffer.cursor.col, 0);
        }
    }

    #[test]
    fn test_paragraph_backward_basic() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with two paragraphs separated by an empty line
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec![
                "first paragraph".to_string(),
                "line two".to_string(),
                "".to_string(), // empty line separating paragraphs
                "second paragraph".to_string(),
                "another line".to_string(),
            ];
            buffer.cursor.row = 4; // Start in second paragraph
            buffer.cursor.col = 5;
        }

        let result = handler.action_paragraph_backward(&mut editor);
        assert!(result.is_ok());

        // Should move to start of first paragraph
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 0);
        }
    }

    #[test]
    fn test_paragraph_backward_multiple_empty_lines() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with multiple empty lines between paragraphs
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec![
                "first paragraph".to_string(),
                "".to_string(),
                "".to_string(), // multiple empty lines
                "".to_string(),
                "second paragraph".to_string(),
                "line two".to_string(),
            ];
            buffer.cursor.row = 5; // Start in second paragraph
            buffer.cursor.col = 0;
        }

        let result = handler.action_paragraph_backward(&mut editor);
        assert!(result.is_ok());

        // Should move to start of first paragraph
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 0);
        }
    }

    #[test]
    fn test_paragraph_backward_from_beginning() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer and start from the first paragraph
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec![
                "first paragraph".to_string(),
                "line two".to_string(),
                "".to_string(),
                "second paragraph".to_string(),
            ];
            buffer.cursor.row = 1; // Start in first paragraph
            buffer.cursor.col = 3;
        }

        let result = handler.action_paragraph_backward(&mut editor);
        assert!(result.is_ok());

        // Should stay at start of first paragraph
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 0);
        }
    }

    #[test]
    fn test_paragraph_movement_three_paragraphs() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with three paragraphs
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec![
                "first paragraph".to_string(),
                "first continued".to_string(),
                "".to_string(),
                "second paragraph".to_string(),
                "second continued".to_string(),
                "".to_string(),
                "third paragraph".to_string(),
                "third continued".to_string(),
            ];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        // Move forward through paragraphs
        let result = handler.action_paragraph_forward(&mut editor);
        assert!(result.is_ok());
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 3); // second paragraph
        }

        let result = handler.action_paragraph_forward(&mut editor);
        assert!(result.is_ok());
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 6); // third paragraph
        }

        // Move backward through paragraphs
        let result = handler.action_paragraph_backward(&mut editor);
        assert!(result.is_ok());
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 3); // back to second paragraph
        }

        let result = handler.action_paragraph_backward(&mut editor);
        assert!(result.is_ok());
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0); // back to first paragraph
        }
    }

    #[test]
    fn test_paragraph_movement_whitespace_only_lines() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with whitespace-only lines (should be treated as empty)
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec![
                "first paragraph".to_string(),
                "   ".to_string(), // whitespace only
                "\t".to_string(),  // tab only
                "second paragraph".to_string(),
            ];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        let result = handler.action_paragraph_forward(&mut editor);
        assert!(result.is_ok());

        // Should move to second paragraph (whitespace lines treated as empty)
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 3);
            assert_eq!(buffer.cursor.col, 0);
        }
    }

    #[test]
    fn test_paragraph_movement_single_line_buffer() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with only one line
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["single line".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 5;
        }

        // Forward movement should stay at the same position
        let result = handler.action_paragraph_forward(&mut editor);
        assert!(result.is_ok());
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 0);
        }

        // Backward movement should also stay at the same position
        let result = handler.action_paragraph_backward(&mut editor);
        assert!(result.is_ok());
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 0);
        }
    }

    #[test]
    fn test_paragraph_movement_all_empty_lines() {
        let handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with only empty lines
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["".to_string(), "".to_string(), "".to_string()];
            buffer.cursor.row = 1;
            buffer.cursor.col = 0;
        }

        // Forward movement should go to last line
        let result = handler.action_paragraph_forward(&mut editor);
        assert!(result.is_ok());
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 2);
            assert_eq!(buffer.cursor.col, 0);
        }

        // Backward movement should go to first line
        let result = handler.action_paragraph_backward(&mut editor);
        assert!(result.is_ok());
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 0);
        }
    }

    // Repeat command tests
    #[test]
    fn test_repeat_delete_char() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with some text
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello world".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        // Delete first character using the full execution path
        let key = create_key_event(KeyCode::Char('x'));
        let result = handler.execute_action(&mut editor, "delete_char_at_cursor", key);
        assert!(result.is_ok());

        // Verify character was deleted
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines[0], "ello world");
            assert_eq!(buffer.cursor.col, 0);
        }

        // Repeat the delete operation using the full execution path
        let repeat_key = create_key_event(KeyCode::Char('.'));
        let result = handler.execute_action(&mut editor, "repeat_last_change", repeat_key);
        assert!(result.is_ok());

        // Verify another character was deleted
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines[0], "llo world");
            assert_eq!(buffer.cursor.col, 0);
        }
    }

    #[test]
    fn test_repeat_substitute_char() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with some text
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["test line".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        // Substitute first character using the full execution path
        let key = create_key_event(KeyCode::Char('s'));
        let result = handler.execute_action(&mut editor, "substitute_char", key);
        assert!(result.is_ok());

        // Verify we're in insert mode and character was deleted
        assert_eq!(editor.mode(), Mode::Insert);
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines[0], "est line");
            assert_eq!(buffer.cursor.col, 0);
        }

        // Return to normal mode and move cursor
        editor.set_mode(Mode::Normal);
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.cursor.col = 1; // Move to second character
        }

        // Repeat the substitute operation using the full execution path
        let repeat_key = create_key_event(KeyCode::Char('.'));
        let result = handler.execute_action(&mut editor, "repeat_last_change", repeat_key);
        assert!(result.is_ok());

        // Verify substitute was repeated
        assert_eq!(editor.mode(), Mode::Insert);
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines[0], "et line");
            assert_eq!(buffer.cursor.col, 1);
        }
    }

    #[test]
    fn test_repeat_delete_line() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with multiple lines
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec![
                "line 1".to_string(),
                "line 2".to_string(),
                "line 3".to_string(),
                "line 4".to_string(),
            ];
            buffer.cursor.row = 1;
            buffer.cursor.col = 0;
        }

        // Delete line 2 using the full execution path
        let key = create_key_event(KeyCode::Char('d')); // This would be 'dd' in real usage
        let result = handler.execute_action(&mut editor, "delete_line", key);
        assert!(result.is_ok());

        // Verify line was deleted
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines.len(), 3);
            assert_eq!(buffer.lines[1], "line 3");
            assert_eq!(buffer.cursor.row, 1);
        }

        // Repeat the delete line operation using the full execution path
        let repeat_key = create_key_event(KeyCode::Char('.'));
        let result = handler.execute_action(&mut editor, "repeat_last_change", repeat_key);
        assert!(result.is_ok());

        // Verify another line was deleted
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines.len(), 2);
            assert_eq!(buffer.lines[1], "line 4");
            assert_eq!(buffer.cursor.row, 1);
        }
    }

    #[test]
    fn test_repeat_join_lines() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer with multiple lines
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec![
                "first".to_string(),
                "second".to_string(),
                "third".to_string(),
                "fourth".to_string(),
            ];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        // Join first and second lines using the full execution path
        let key = create_key_event(KeyCode::Char('J'));
        let result = handler.execute_action(&mut editor, "join_lines", key);
        assert!(result.is_ok());

        // Verify lines were joined
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines.len(), 3);
            assert_eq!(buffer.lines[0], "first second");
            assert_eq!(buffer.cursor.row, 0);
        }

        // Repeat the join operation using the full execution path
        let repeat_key = create_key_event(KeyCode::Char('.'));
        let result = handler.execute_action(&mut editor, "repeat_last_change", repeat_key);
        assert!(result.is_ok());

        // Verify another join occurred
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines.len(), 2);
            assert_eq!(buffer.lines[0], "first second third");
            assert_eq!(buffer.cursor.row, 0);
        }
    }

    #[test]
    fn test_repeat_without_previous_command() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["test".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        // Try to repeat without any previous command using the full execution path
        let repeat_key = create_key_event(KeyCode::Char('.'));
        let result = handler.execute_action(&mut editor, "repeat_last_change", repeat_key);
        assert!(result.is_ok());

        // Buffer should remain unchanged
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines[0], "test");
            assert_eq!(buffer.cursor.row, 0);
            assert_eq!(buffer.cursor.col, 0);
        }
    }

    #[test]
    fn test_repeat_non_repeatable_command() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["test".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 2;
        }

        // Execute a movement command (non-repeatable) using the full execution path
        let key = create_key_event(KeyCode::Char('h'));
        let result = handler.execute_action(&mut editor, "cursor_left", key);
        assert!(result.is_ok());

        // Verify cursor moved
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 1);
        }

        // Try to repeat - should have no effect since movement isn't repeatable
        let repeat_key = create_key_event(KeyCode::Char('.'));
        let result = handler.execute_action(&mut editor, "repeat_last_change", repeat_key);
        assert!(result.is_ok());

        // Cursor should remain at current position
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.cursor.col, 1);
        }
    }

    #[test]
    fn test_repeat_recording_and_replay() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["abcdef".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        // Execute a repeatable command via the full flow
        let key = create_key_event(KeyCode::Char('x'));
        let result = handler.execute_action(&mut editor, "delete_char_at_cursor", key);
        assert!(result.is_ok());

        // Verify the command was recorded
        assert!(handler.last_command.is_some());
        if let Some(ref cmd) = handler.last_command {
            assert_eq!(cmd.action, "delete_char_at_cursor");
        }

        // Verify first deletion
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines[0], "bcdef");
            assert_eq!(buffer.cursor.col, 0);
        }

        // Execute repeat command
        let repeat_key = create_key_event(KeyCode::Char('.'));
        let result = handler.execute_action(&mut editor, "repeat_last_change", repeat_key);
        assert!(result.is_ok());

        // Verify repeat worked
        if let Some(buffer) = editor.current_buffer() {
            assert_eq!(buffer.lines[0], "cdef");
            assert_eq!(buffer.cursor.col, 0);
        }
    }

    #[test]
    fn test_repeat_put_operations() {
        let mut handler = KeyHandler::new();
        let mut editor = create_test_editor();

        // Create a buffer and yank some text first
        editor.create_buffer(None).expect("Failed to create buffer");
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.lines = vec!["hello world".to_string()];
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }

        // Yank a word first using the full execution path
        let yank_key = create_key_event(KeyCode::Char('y'));
        let result = handler.execute_action(&mut editor, "yank_word", yank_key);
        assert!(result.is_ok());

        // Move cursor to end of line and put the yanked text using the full execution path
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.cursor.col = 11; // Move to end: "hello world"
        }

        let put_key = create_key_event(KeyCode::Char('p'));
        let result = handler.execute_action(&mut editor, "put_after", put_key);
        assert!(result.is_ok());

        // Verify put operation worked
        if let Some(buffer) = editor.current_buffer() {
            // Should be "hello worldhello" with cursor after the pasted text
            assert!(buffer.lines[0].contains("hello world"));
            assert!(buffer.lines[0].len() > 11); // Should be longer than original
        }

        // Repeat the put operation using the full execution path
        let repeat_key = create_key_event(KeyCode::Char('.'));
        let result = handler.execute_action(&mut editor, "repeat_last_change", repeat_key);
        assert!(result.is_ok());

        // Verify repeat worked - should have another "hello" added
        if let Some(buffer) = editor.current_buffer() {
            let line = &buffer.lines[0];
            let hello_count = line.matches("hello").count();
            assert_eq!(hello_count, 3); // Original + first put + repeat put
        }
    }
}
