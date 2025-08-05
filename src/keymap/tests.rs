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
}
