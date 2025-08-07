// Event-driven editor implementation
// This provides concrete event handlers for the oxidized editor

use crate::Editor;
use crate::event::*;
use crate::mode::Mode;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use log::{debug, info, trace, warn};
use std::sync::{Arc, Mutex, mpsc};

/// Input event handler - processes keyboard input and mode changes
pub struct InputEventHandler {
    /// Reference to the main editor state
    editor: Arc<Mutex<Editor>>,
}

impl InputEventHandler {
    pub fn new(editor: Arc<Mutex<Editor>>) -> Self {
        Self { editor }
    }

    /// Convert raw key events to higher-level input events
    #[allow(dead_code)] // Will be used when full event-driven key handling is implemented
    fn process_key_event(&self, key_event: KeyEvent) -> Vec<EditorEvent> {
        let mut events = Vec::new();

        if let Ok(mut editor) = self.editor.lock() {
            let current_mode = editor.mode();

            match current_mode {
                Mode::Normal => {
                    events.extend(self.handle_normal_mode_key(&mut editor, key_event));
                }
                Mode::Insert => {
                    events.extend(self.handle_insert_mode_key(&mut editor, key_event));
                }
                Mode::Command => {
                    events.extend(self.handle_command_mode_key(&mut editor, key_event));
                }
                Mode::Visual => {
                    events.extend(self.handle_visual_mode_key(&mut editor, key_event));
                }
                Mode::Search => {
                    events.extend(self.handle_search_mode_key(&mut editor, key_event));
                }
                _ => {}
            }
        }

        events
    }

    #[allow(dead_code)] // Will be used when full event-driven key handling is implemented
    fn handle_normal_mode_key(&self, editor: &mut Editor, key: KeyEvent) -> Vec<EditorEvent> {
        let mut events = Vec::new();

        match (key.code, key.modifiers) {
            // Mode transitions
            (KeyCode::Char('i'), KeyModifiers::NONE) => {
                events.push(EditorEvent::Input(InputEvent::ModeChange {
                    from: Mode::Normal,
                    to: Mode::Insert,
                }));
            }
            (KeyCode::Char(':'), KeyModifiers::NONE) => {
                events.push(EditorEvent::Input(InputEvent::ModeChange {
                    from: Mode::Normal,
                    to: Mode::Command,
                }));
            }
            (KeyCode::Char('/'), KeyModifiers::NONE) => {
                events.push(EditorEvent::Input(InputEvent::ModeChange {
                    from: Mode::Normal,
                    to: Mode::Search,
                }));
            }
            (KeyCode::Char('v'), KeyModifiers::NONE) => {
                events.push(EditorEvent::Input(InputEvent::ModeChange {
                    from: Mode::Normal,
                    to: Mode::Visual,
                }));
            }

            // Movement commands generate cursor events
            (KeyCode::Char('j'), KeyModifiers::NONE) => {
                if let Some(buffer) = editor.current_buffer() {
                    let old_pos = (buffer.cursor.row, buffer.cursor.col);
                    // Move cursor down logic would go here
                    let new_pos = (buffer.cursor.row + 1, buffer.cursor.col);
                    events.push(EditorEvent::Buffer(BufferEvent::CursorMoved {
                        buffer_id: 0, // Would get actual buffer ID
                        old_pos,
                        new_pos,
                    }));
                }
            }
            (KeyCode::Char('k'), KeyModifiers::NONE) => {
                if let Some(buffer) = editor.current_buffer() {
                    let old_pos = (buffer.cursor.row, buffer.cursor.col);
                    let new_pos = (buffer.cursor.row.saturating_sub(1), buffer.cursor.col);
                    events.push(EditorEvent::Buffer(BufferEvent::CursorMoved {
                        buffer_id: 0, // Would get actual buffer ID
                        old_pos,
                        new_pos,
                    }));
                }
            }

            // Window management
            (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
                events.push(EditorEvent::Window(WindowEvent::Split {
                    parent_id: 0,     // Current window
                    new_window_id: 1, // Would be generated
                    direction: crate::window::SplitDirection::Horizontal,
                }));
            }

            // File operations
            (KeyCode::Char('s'), KeyModifiers::NONE) => {
                // Save current buffer - would need to get actual buffer ID
                events.push(EditorEvent::Buffer(BufferEvent::Modified { buffer_id: 0 }));
            }

            _ => {
                // Pass through to existing key handler for now
                trace!("Unhandled normal mode key: {:?}", key);
            }
        }

        events
    }

    #[allow(dead_code)] // Will be used when full event-driven key handling is implemented
    fn handle_insert_mode_key(&self, editor: &mut Editor, key: KeyEvent) -> Vec<EditorEvent> {
        let mut events = Vec::new();

        match (key.code, key.modifiers) {
            (KeyCode::Esc, KeyModifiers::NONE) => {
                events.push(EditorEvent::Input(InputEvent::ModeChange {
                    from: Mode::Insert,
                    to: Mode::Normal,
                }));
            }
            (KeyCode::Char(c), KeyModifiers::NONE) => {
                if let Some(buffer) = editor.current_buffer() {
                    events.push(EditorEvent::Input(InputEvent::TextInsert {
                        text: c.to_string(),
                        position: (buffer.cursor.row, buffer.cursor.col),
                    }));
                }
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                if let Some(buffer) = editor.current_buffer() {
                    events.push(EditorEvent::Input(InputEvent::TextInsert {
                        text: "\n".to_string(),
                        position: (buffer.cursor.row, buffer.cursor.col),
                    }));
                }
            }
            _ => {
                trace!("Unhandled insert mode key: {:?}", key);
            }
        }

        events
    }

    #[allow(dead_code)] // Will be used when full event-driven key handling is implemented
    fn handle_command_mode_key(&self, editor: &mut Editor, key: KeyEvent) -> Vec<EditorEvent> {
        let mut events = Vec::new();

        match (key.code, key.modifiers) {
            (KeyCode::Esc, KeyModifiers::NONE) => {
                events.push(EditorEvent::Input(InputEvent::ModeChange {
                    from: Mode::Command,
                    to: Mode::Normal,
                }));
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                let command = editor.command_line().to_string();
                events.push(EditorEvent::Input(InputEvent::Command(command)));
                events.push(EditorEvent::Input(InputEvent::ModeChange {
                    from: Mode::Command,
                    to: Mode::Normal,
                }));
            }
            (KeyCode::Char(c), KeyModifiers::NONE) => {
                events.push(EditorEvent::UI(UIEvent::CommandLineUpdated(format!(
                    "{}{}",
                    editor.command_line(),
                    c
                ))));
            }
            _ => {
                trace!("Unhandled command mode key: {:?}", key);
            }
        }

        events
    }

    #[allow(dead_code)] // Will be used when full event-driven key handling is implemented
    fn handle_visual_mode_key(&self, _editor: &mut Editor, key: KeyEvent) -> Vec<EditorEvent> {
        let mut events = Vec::new();

        match (key.code, key.modifiers) {
            (KeyCode::Esc, KeyModifiers::NONE) => {
                events.push(EditorEvent::Input(InputEvent::ModeChange {
                    from: Mode::Visual,
                    to: Mode::Normal,
                }));
            }
            _ => {
                trace!("Unhandled visual mode key: {:?}", key);
            }
        }

        events
    }

    #[allow(dead_code)] // Will be used when full event-driven key handling is implemented
    fn handle_search_mode_key(&self, _editor: &mut Editor, key: KeyEvent) -> Vec<EditorEvent> {
        let mut events = Vec::new();

        match (key.code, key.modifiers) {
            (KeyCode::Esc, KeyModifiers::NONE) => {
                events.push(EditorEvent::Search(SearchEvent::Cancelled));
                events.push(EditorEvent::Input(InputEvent::ModeChange {
                    from: Mode::Search,
                    to: Mode::Normal,
                }));
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                // Start search with current pattern
                events.push(EditorEvent::Search(SearchEvent::Started {
                    pattern: "".to_string(), // Would get from search buffer
                    is_regex: false,
                }));
            }
            _ => {
                trace!("Unhandled search mode key: {:?}", key);
            }
        }

        events
    }
}

impl EventHandler for InputEventHandler {
    fn handle_event(&mut self, event: &EditorEvent) -> EventResult {
        match event {
            EditorEvent::Input(InputEvent::KeyPress(_key_event)) => {
                // For now, just trigger a redraw when keys are pressed
                // The actual key handling will be done through the existing system
                EventResult::Events(vec![EditorEvent::UI(UIEvent::RedrawRequest)])
            }
            EditorEvent::Input(InputEvent::ModeChange { from: _, to }) => {
                debug!("Mode transition to: {:?}", to);
                if let Ok(mut editor) = self.editor.lock() {
                    editor.set_mode(*to);
                    EventResult::Events(vec![EditorEvent::UI(UIEvent::RedrawRequest)])
                } else {
                    EventResult::Error("Failed to lock editor".to_string())
                }
            }
            EditorEvent::Input(InputEvent::Command(command)) => {
                info!("Processing command: '{}'", command);
                if let Ok(mut editor) = self.editor.lock() {
                    // Set the command line and execute it
                    let cleaned_command = command.trim_start_matches(':').to_string();

                    // Execute command using our simplified command processor
                    match Self::execute_editor_command(&mut editor, &cleaned_command) {
                        Ok(message) => EventResult::Events(vec![
                            EditorEvent::UI(UIEvent::StatusMessage(message)),
                            EditorEvent::UI(UIEvent::RedrawRequest),
                        ]),
                        Err(e) => EventResult::Events(vec![
                            EditorEvent::UI(UIEvent::StatusMessage(format!("Error: {}", e))),
                            EditorEvent::UI(UIEvent::RedrawRequest),
                        ]),
                    }
                } else {
                    EventResult::Error("Failed to lock editor".to_string())
                }
            }
            _ => EventResult::NotHandled,
        }
    }

    fn can_handle(&self, event: &EditorEvent) -> bool {
        matches!(event, EditorEvent::Input(_))
    }
}

impl InputEventHandler {
    /// Execute editor commands (simplified version of KeyHandler's action_execute_command)
    fn execute_editor_command(editor: &mut Editor, command: &str) -> Result<String, String> {
        match command {
            "q" | "quit" => {
                editor.quit();
                Ok("Quitting editor".to_string())
            }
            "q!" | "quit!" => {
                editor.force_quit();
                Ok("Force quit".to_string())
            }
            "w" | "write" => {
                if let Some(buffer) = editor.current_buffer_mut() {
                    match buffer.save() {
                        Ok(_) => Ok("File saved".to_string()),
                        Err(e) => Err(format!("Error saving: {}", e)),
                    }
                } else {
                    Err("No buffer to save".to_string())
                }
            }
            "wq" | "x" => {
                if let Some(buffer) = editor.current_buffer_mut() {
                    match buffer.save() {
                        Ok(_) => {
                            editor.quit();
                            Ok("File saved and exiting".to_string())
                        }
                        Err(e) => Err(format!("Error saving: {}", e)),
                    }
                } else {
                    editor.quit();
                    Ok("Exiting".to_string())
                }
            }
            _ => {
                if command.starts_with("e ") {
                    let filename = command[2..].trim();
                    match editor.open_file(filename) {
                        Ok(msg) => Ok(msg),
                        Err(e) => Err(format!("Error opening file: {}", e)),
                    }
                } else {
                    Err(format!("Unknown command: {}", command))
                }
            }
        }
    }
}

/// Buffer event handler - manages buffer operations
pub struct BufferEventHandler {
    _editor: Arc<Mutex<Editor>>, // Prefix with underscore to avoid unused warning
}

impl BufferEventHandler {
    pub fn new(editor: Arc<Mutex<Editor>>) -> Self {
        Self { _editor: editor }
    }
}

impl EventHandler for BufferEventHandler {
    fn handle_event(&mut self, event: &EditorEvent) -> EventResult {
        match event {
            EditorEvent::Buffer(BufferEvent::ContentChanged { buffer_id, .. }) => {
                debug!("Buffer {} content changed", buffer_id);
                // Trigger syntax highlighting, mark as modified, etc.
                let events = vec![
                    EditorEvent::Buffer(BufferEvent::Modified {
                        buffer_id: *buffer_id,
                    }),
                    EditorEvent::UI(UIEvent::RedrawRequest),
                ];
                EventResult::Events(events)
            }
            EditorEvent::Buffer(BufferEvent::CursorMoved {
                buffer_id,
                old_pos,
                new_pos,
            }) => {
                trace!(
                    "Cursor moved in buffer {}: {:?} -> {:?}",
                    buffer_id, old_pos, new_pos
                );
                // Update cursor position, trigger viewport updates if needed
                EventResult::Events(vec![EditorEvent::UI(UIEvent::RedrawRequest)])
            }
            EditorEvent::Buffer(BufferEvent::Modified { buffer_id }) => {
                debug!("Buffer {} marked as modified", buffer_id);
                // Buffer modification handled by editor
                EventResult::Handled
            }
            _ => EventResult::NotHandled,
        }
    }

    fn can_handle(&self, event: &EditorEvent) -> bool {
        matches!(event, EditorEvent::Buffer(_))
    }
}

/// UI event handler - manages rendering and display
pub struct UIEventHandler {
    editor: Arc<Mutex<Editor>>,
    needs_redraw: bool,
}

impl UIEventHandler {
    pub fn new(editor: Arc<Mutex<Editor>>) -> Self {
        Self {
            editor,
            needs_redraw: false,
        }
    }
}

impl EventHandler for UIEventHandler {
    fn handle_event(&mut self, event: &EditorEvent) -> EventResult {
        match event {
            EditorEvent::UI(UIEvent::RedrawRequest) => {
                self.needs_redraw = true;
                trace!("Redraw requested");
                EventResult::Handled
            }
            EditorEvent::UI(UIEvent::Resize { width, height }) => {
                info!("Terminal resized to {}x{}", width, height);
                // Terminal resize handled by editor
                self.needs_redraw = true;
                EventResult::Handled
            }
            EditorEvent::UI(UIEvent::ThemeChanged(theme)) => {
                info!("Theme changed to: {}", theme);
                // Theme change handled by editor
                self.needs_redraw = true;
                EventResult::Handled
            }
            EditorEvent::UI(UIEvent::StatusMessage(message)) => {
                if let Ok(mut editor) = self.editor.lock() {
                    editor.set_status_message(message.clone());
                }
                self.needs_redraw = true;
                EventResult::Handled
            }
            EditorEvent::UI(UIEvent::CommandLineUpdated(text)) => {
                if let Ok(mut editor) = self.editor.lock() {
                    editor.set_command_line(text.clone());
                }
                self.needs_redraw = true;
                EventResult::Handled
            }
            _ => EventResult::NotHandled,
        }
    }

    fn can_handle(&self, event: &EditorEvent) -> bool {
        matches!(event, EditorEvent::UI(_))
    }
}

/// Configuration event handler
pub struct ConfigEventHandler {
    _editor: Arc<Mutex<Editor>>, // Prefix with underscore to avoid unused warning
}

impl ConfigEventHandler {
    pub fn new(editor: Arc<Mutex<Editor>>) -> Self {
        Self { _editor: editor }
    }
}

impl EventHandler for ConfigEventHandler {
    fn handle_event(&mut self, event: &EditorEvent) -> EventResult {
        match event {
            EditorEvent::Config(ConfigEvent::EditorConfigChanged) => {
                info!("Editor configuration changed, reloading");
                // Configuration reload handled by editor
                EventResult::Events(vec![EditorEvent::UI(UIEvent::RedrawRequest)])
            }
            EditorEvent::Config(ConfigEvent::ThemeConfigChanged) => {
                info!("Theme configuration changed, reloading");
                EventResult::Events(vec![
                    EditorEvent::UI(UIEvent::RedrawRequest),
                    EditorEvent::UI(UIEvent::StatusMessage("Theme reloaded".to_string())),
                ])
            }
            EditorEvent::Config(ConfigEvent::SettingChanged { key, value }) => {
                info!("Setting changed: {} = {}", key, value);
                // Apply setting change
                EventResult::Handled
            }
            _ => EventResult::NotHandled,
        }
    }

    fn can_handle(&self, event: &EditorEvent) -> bool {
        matches!(event, EditorEvent::Config(_))
    }
}

/// System event handler
pub struct SystemEventHandler {
    should_quit: Arc<Mutex<bool>>,
}

impl SystemEventHandler {
    pub fn new(should_quit: Arc<Mutex<bool>>) -> Self {
        Self { should_quit }
    }
}

impl EventHandler for SystemEventHandler {
    fn handle_event(&mut self, event: &EditorEvent) -> EventResult {
        match event {
            EditorEvent::System(SystemEvent::Quit) => {
                info!("Quit event received");
                if let Ok(mut quit) = self.should_quit.lock() {
                    *quit = true;
                }
                EventResult::Handled
            }
            EditorEvent::System(SystemEvent::FileChanged(path)) => {
                warn!("File changed externally: {:?}", path);
                // Handle external file changes
                EventResult::Events(vec![
                    EditorEvent::UI(UIEvent::StatusMessage(format!(
                        "File {} changed externally",
                        path.display()
                    ))),
                    EditorEvent::UI(UIEvent::RedrawRequest),
                ])
            }
            _ => EventResult::NotHandled,
        }
    }

    fn can_handle(&self, event: &EditorEvent) -> bool {
        matches!(event, EditorEvent::System(_))
    }
}

/// Event-driven editor that replaces the traditional main loop
pub struct EventDrivenEditor {
    /// Event bus for managing all events
    event_bus: EventBus,
    /// Event dispatcher for routing events
    dispatcher: EventDispatcher,
    /// Should quit flag
    should_quit: Arc<Mutex<bool>>,
    /// Reference to the main editor (for compatibility)
    editor: Arc<Mutex<Editor>>,
}

impl EventDrivenEditor {
    pub fn new(editor: Editor) -> Self {
        let editor = Arc::new(Mutex::new(editor));
        let should_quit = Arc::new(Mutex::new(false));
        let event_bus = EventBus::new();
        let mut dispatcher = EventDispatcher::new();

        // Register event handlers
        dispatcher.register_input_handler(Box::new(InputEventHandler::new(editor.clone())));
        dispatcher.register_buffer_handler(Box::new(BufferEventHandler::new(editor.clone())));
        dispatcher.register_ui_handler(Box::new(UIEventHandler::new(editor.clone())));
        dispatcher.register_config_handler(Box::new(ConfigEventHandler::new(editor.clone())));
        dispatcher.register_system_handler(Box::new(SystemEventHandler::new(should_quit.clone())));

        Self {
            event_bus,
            dispatcher,
            should_quit,
            editor,
        }
    }

    /// Main event loop - replaces the traditional Editor::run() method
    pub fn run(&mut self) -> Result<(), anyhow::Error> {
        info!("Starting event-driven editor");

        // Initialize editor if no buffers exist
        if let Ok(mut editor) = self.editor.lock() {
            if editor.current_buffer().is_none() {
                debug!("No buffers exist, creating initial empty buffer");
                if let Err(e) = editor.create_buffer(None) {
                    warn!("Failed to create initial buffer: {}", e);
                }
            }
        }

        // Initial render event
        self.event_bus
            .send(EditorEvent::UI(UIEvent::RedrawRequest))
            .map_err(|e| anyhow::anyhow!("Failed to send initial render event: {}", e))?;

        loop {
            // Check quit condition
            if let Ok(quit) = self.should_quit.lock() {
                if *quit {
                    info!("Quit requested, exiting event loop");
                    break;
                }
            }

            // Check for terminal input events
            if crossterm::event::poll(std::time::Duration::from_millis(16))
                .map_err(|e| anyhow::anyhow!("Failed to poll terminal events: {}", e))?
            {
                match crossterm::event::read() {
                    Ok(crossterm::event::Event::Key(key_event)) => {
                        self.event_bus
                            .send(EditorEvent::Input(InputEvent::KeyPress(key_event)))
                            .map_err(|e| anyhow::anyhow!("Failed to send key event: {}", e))?;
                    }
                    Ok(crossterm::event::Event::Resize(width, height)) => {
                        self.event_bus
                            .send(EditorEvent::UI(UIEvent::Resize { width, height }))
                            .map_err(|e| anyhow::anyhow!("Failed to send resize event: {}", e))?;
                    }
                    Ok(_) => {} // Ignore other events
                    Err(e) => {
                        warn!("Failed to read terminal event: {}", e);
                    }
                }
            }

            // Process all pending events
            let mut events_to_process = self.event_bus.process_events();

            // Process generated events until queue is empty
            while !events_to_process.is_empty() {
                let mut new_events = Vec::new();

                for event in events_to_process {
                    let generated = self.dispatcher.dispatch(&event);
                    new_events.extend(generated);
                }

                events_to_process = new_events;
            }

            // Small delay to prevent busy waiting
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        info!("Event-driven editor loop completed");
        Ok(())
    }

    /// Send an event to the editor
    pub fn send_event(&self, event: EditorEvent) -> Result<(), mpsc::SendError<EditorEvent>> {
        self.event_bus.send(event)
    }

    /// Get event sender for external components
    pub fn event_sender(&self) -> mpsc::Sender<EditorEvent> {
        self.event_bus.sender()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Editor;

    #[test]
    fn test_event_driven_editor_creation() -> Result<(), Box<dyn std::error::Error>> {
        let editor = Editor::new()?;
        let _event_driven = EventDrivenEditor::new(editor);
        // Test that we can create the event-driven editor
        Ok(())
    }

    #[test]
    fn test_event_sending() -> Result<(), Box<dyn std::error::Error>> {
        let editor = Editor::new()?;
        let event_driven = EventDrivenEditor::new(editor);

        // Test sending events
        event_driven.send_event(EditorEvent::UI(UIEvent::RedrawRequest))?;
        event_driven.send_event(EditorEvent::System(SystemEvent::Quit))?;

        Ok(())
    }
}
