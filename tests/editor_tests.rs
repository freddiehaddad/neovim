#[cfg(test)]
mod editor_tests {
    use oxidized::core::editor::PendingOperator;
    use oxidized::core::{Editor, mode::Mode};

    #[test]
    fn test_editor_creation() {
        let editor = Editor::new();
        assert!(editor.is_ok());

        let editor = editor.unwrap();
        assert_eq!(editor.mode(), Mode::Normal);
        assert_eq!(editor.command_line(), "");
        assert_eq!(editor.status_message(), "");
    }

    #[test]
    fn test_editor_mode_transitions() {
        let mut editor = Editor::new().unwrap();

        // Start in Normal mode
        assert_eq!(editor.mode(), Mode::Normal);

        // Change to Insert mode
        editor.set_mode(Mode::Insert);
        assert_eq!(editor.mode(), Mode::Insert);

        // Change to Command mode
        editor.set_mode(Mode::Command);
        assert_eq!(editor.mode(), Mode::Command);

        // Back to Normal mode
        editor.set_mode(Mode::Normal);
        assert_eq!(editor.mode(), Mode::Normal);
    }

    #[test]
    fn test_command_line_operations() {
        let mut editor = Editor::new().unwrap();

        // Test setting command line
        editor.set_command_line(":quit".to_string());
        assert_eq!(editor.command_line(), ":quit");

        // Test clearing command line
        editor.set_command_line(String::new());
        assert_eq!(editor.command_line(), "");
    }

    #[test]
    fn test_status_messages() {
        let mut editor = Editor::new().unwrap();

        // Test setting status message
        editor.set_status_message("File saved".to_string());
        assert_eq!(editor.status_message(), "File saved");

        // Test updating status message
        editor.set_status_message("Ready".to_string());
        assert_eq!(editor.status_message(), "Ready");
    }

    #[test]
    fn test_pending_operator() {
        let mut editor = Editor::new().unwrap();

        // Initially no pending operator
        assert!(editor.get_pending_operator().is_none());

        // Set a pending operator
        editor.set_pending_operator(PendingOperator::Delete);
        assert!(editor.get_pending_operator().is_some());
        assert_eq!(editor.mode(), Mode::OperatorPending);

        // Clear pending operator
        editor.clear_pending_operator();
        assert!(editor.get_pending_operator().is_none());
    }

    #[test]
    fn test_buffer_operations() {
        let mut editor = Editor::new().unwrap();

        // New editor should start with no buffers
        assert!(editor.current_buffer().is_none());

        // Create a new buffer
        let buffer_id = editor.create_buffer(None).unwrap();
        assert!(editor.current_buffer().is_some());

        // Test buffer list
        let buffer_list = editor.list_buffers();
        assert!(buffer_list.contains(&buffer_id.to_string()));
    }

    #[test]
    fn test_completion_system() {
        let mut editor = Editor::new().unwrap();

        // Initially completion should not be active
        assert!(!editor.is_completion_active());

        // Start completion
        editor.start_command_completion("se");

        // Should have matches for "set" commands
        assert!(editor.completion_has_matches());
    }
}
