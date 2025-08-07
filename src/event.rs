// Event-driven architecture for the oxidized editor
// This module provides the foundation for converting from the current imperative model
// to an event-driven architecture like Neovim and Vim

use crate::mode::Mode;
use crate::search::SearchResult;
use crossterm::event::KeyEvent;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

/// Core event types that can occur in the editor
#[derive(Debug, Clone)]
pub enum EditorEvent {
    /// Input events
    Input(InputEvent),
    /// Buffer events
    Buffer(BufferEvent),
    /// UI events  
    UI(UIEvent),
    /// Window events
    Window(WindowEvent),
    /// Configuration events
    Config(ConfigEvent),
    /// Search events
    Search(SearchEvent),
    /// System events
    System(SystemEvent),
    /// Plugin events (future extensibility)
    Plugin(PluginEvent),
    /// LSP events (future LSP integration)
    LSP(LSPEvent),
}

/// Input-related events
#[derive(Debug, Clone)]
pub enum InputEvent {
    /// Raw key press from terminal
    KeyPress(KeyEvent),
    /// Parsed key sequence (e.g., "gg", "dd", "2yy")
    KeySequence(String),
    /// Mode transition request
    ModeChange { from: Mode, to: Mode },
    /// Command entered in command mode
    Command(String),
    /// Text insertion in insert mode
    TextInsert {
        text: String,
        position: (usize, usize),
    },
}

/// Buffer-related events
#[derive(Debug, Clone)]
pub enum BufferEvent {
    /// Buffer created
    Created {
        buffer_id: usize,
        path: Option<PathBuf>,
    },
    /// Buffer opened from file
    Opened { buffer_id: usize, path: PathBuf },
    /// Buffer modified
    Modified { buffer_id: usize },
    /// Buffer saved
    Saved { buffer_id: usize, path: PathBuf },
    /// Buffer closed
    Closed { buffer_id: usize },
    /// Content changed
    ContentChanged {
        buffer_id: usize,
        line: usize,
        col: usize,
        old_text: String,
        new_text: String,
    },
    /// Cursor moved
    CursorMoved {
        buffer_id: usize,
        old_pos: (usize, usize),
        new_pos: (usize, usize),
    },
    /// Selection changed
    SelectionChanged {
        buffer_id: usize,
        start: Option<(usize, usize)>,
        end: Option<(usize, usize)>,
    },
    /// Syntax highlighting updated
    SyntaxHighlighted {
        buffer_id: usize,
        line: usize,
        highlights: Vec<crate::syntax::HighlightRange>,
    },
}

/// UI-related events
#[derive(Debug, Clone)]
pub enum UIEvent {
    /// Terminal resized
    Resize { width: u16, height: u16 },
    /// Redraw requested
    RedrawRequest,
    /// Theme changed
    ThemeChanged(String),
    /// Status message updated
    StatusMessage(String),
    /// Command line updated
    CommandLineUpdated(String),
    /// Viewport changed (scrolling)
    ViewportChanged {
        buffer_id: usize,
        top: usize,
        visible_lines: usize,
    },
}

/// Window management events
#[derive(Debug, Clone)]
pub enum WindowEvent {
    /// Window created
    Created { window_id: usize },
    /// Window closed
    Closed { window_id: usize },
    /// Window split
    Split {
        parent_id: usize,
        new_window_id: usize,
        direction: crate::window::SplitDirection,
    },
    /// Window focus changed
    FocusChanged {
        old_window_id: Option<usize>,
        new_window_id: usize,
    },
    /// Window resized
    Resized {
        window_id: usize,
        width: u16,
        height: u16,
    },
}

/// Configuration events
#[derive(Debug, Clone)]
pub enum ConfigEvent {
    /// Editor config file changed
    EditorConfigChanged,
    /// Theme config file changed
    ThemeConfigChanged,
    /// Keymap config file changed
    KeymapConfigChanged,
    /// Setting changed via :set command
    SettingChanged { key: String, value: String },
}

/// Search-related events
#[derive(Debug, Clone)]
pub enum SearchEvent {
    /// Search started
    Started { pattern: String, is_regex: bool },
    /// Search results found
    ResultsFound(Vec<SearchResult>),
    /// Search navigation (n/N)
    Navigate { direction: SearchDirection },
    /// Search cancelled
    Cancelled,
}

#[derive(Debug, Clone)]
pub enum SearchDirection {
    Forward,
    Backward,
}

/// System-level events
#[derive(Debug, Clone)]
pub enum SystemEvent {
    /// Request to quit the editor
    Quit,
    /// Force quit without saving
    ForceQuit,
    /// File system event (file changed externally)
    FileChanged(PathBuf),
    /// Timer event (for periodic tasks)
    Timer { id: String },
    /// Signal received (SIGTERM, etc.)
    Signal(String),
}

/// Plugin system events (future extensibility)
#[derive(Debug, Clone)]
pub enum PluginEvent {
    /// Plugin loaded
    Loaded(String),
    /// Plugin command
    Command { plugin: String, command: String },
    /// Custom event from plugin
    Custom {
        plugin: String,
        data: serde_json::Value,
    },
}

/// LSP events (future Language Server Protocol integration)
#[derive(Debug, Clone)]
pub enum LSPEvent {
    /// LSP server started
    ServerStarted { language: String },
    /// Diagnostics received
    Diagnostics {
        buffer_id: usize,
        diagnostics: Vec<LSPDiagnostic>,
    },
    /// Completion received
    Completion {
        buffer_id: usize,
        position: (usize, usize),
        items: Vec<LSPCompletionItem>,
    },
    /// Hover information
    Hover {
        buffer_id: usize,
        position: (usize, usize),
        content: String,
    },
}

#[derive(Debug, Clone)]
pub struct LSPDiagnostic {
    pub line: usize,
    pub col: usize,
    pub length: usize,
    pub severity: LSPDiagnosticSeverity,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum LSPDiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Debug, Clone)]
pub struct LSPCompletionItem {
    pub label: String,
    pub kind: LSPCompletionKind,
    pub detail: Option<String>,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone)]
pub enum LSPCompletionKind {
    Text,
    Method,
    Function,
    Constructor,
    Field,
    Variable,
    Class,
    Interface,
    Module,
    Property,
    Unit,
    Value,
    Enum,
    Keyword,
    Snippet,
    Color,
    File,
    Reference,
}

/// Event handler trait for components that can process events
pub trait EventHandler {
    fn handle_event(&mut self, event: &EditorEvent) -> EventResult;
    fn can_handle(&self, event: &EditorEvent) -> bool;
}

/// Result of event processing
#[derive(Debug)]
pub enum EventResult {
    /// Event was handled, no further processing needed
    Handled,
    /// Event was handled, but should continue to other handlers
    PassThrough,
    /// Event was not handled
    NotHandled,
    /// Event handling failed
    Error(String),
    /// Event handling generated new events
    Events(Vec<EditorEvent>),
}

/// Event bus for managing event distribution
pub struct EventBus {
    /// Event queue for asynchronous processing
    sender: mpsc::Sender<EditorEvent>,
    receiver: mpsc::Receiver<EditorEvent>,
    /// Registered event handlers
    handlers: Vec<Box<dyn EventHandler + Send>>,
    /// Event history for debugging
    event_history: Vec<(Instant, EditorEvent)>,
    /// Max history size
    max_history: usize,
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            handlers: Vec::new(),
            event_history: Vec::new(),
            max_history: 1000,
        }
    }

    /// Register an event handler
    pub fn register_handler(&mut self, handler: Box<dyn EventHandler + Send>) {
        self.handlers.push(handler);
    }

    /// Send an event to the event bus
    pub fn send(&self, event: EditorEvent) -> Result<(), mpsc::SendError<EditorEvent>> {
        self.sender.send(event)
    }

    /// Get a sender handle for other threads
    pub fn sender(&self) -> mpsc::Sender<EditorEvent> {
        self.sender.clone()
    }

    /// Process all pending events
    pub fn process_events(&mut self) -> Vec<EditorEvent> {
        let mut generated_events = Vec::new();

        // Process all pending events
        while let Ok(event) = self.receiver.try_recv() {
            // Add to history
            self.event_history.push((Instant::now(), event.clone()));
            if self.event_history.len() > self.max_history {
                self.event_history.remove(0);
            }

            // Process event through handlers
            for handler in &mut self.handlers {
                if handler.can_handle(&event) {
                    match handler.handle_event(&event) {
                        EventResult::Handled => break,
                        EventResult::PassThrough => continue,
                        EventResult::NotHandled => continue,
                        EventResult::Error(err) => {
                            eprintln!("Event handler error: {}", err);
                            continue;
                        }
                        EventResult::Events(mut events) => {
                            generated_events.append(&mut events);
                            continue;
                        }
                    }
                }
            }
        }

        generated_events
    }

    /// Get recent event history for debugging
    pub fn get_history(&self, count: usize) -> &[(Instant, EditorEvent)] {
        let start = self.event_history.len().saturating_sub(count);
        &self.event_history[start..]
    }
}

/// Event dispatcher for routing events to appropriate handlers
pub struct EventDispatcher {
    /// Event handlers organized by event type
    input_handlers: Vec<Box<dyn EventHandler + Send>>,
    buffer_handlers: Vec<Box<dyn EventHandler + Send>>,
    ui_handlers: Vec<Box<dyn EventHandler + Send>>,
    window_handlers: Vec<Box<dyn EventHandler + Send>>,
    config_handlers: Vec<Box<dyn EventHandler + Send>>,
    search_handlers: Vec<Box<dyn EventHandler + Send>>,
    system_handlers: Vec<Box<dyn EventHandler + Send>>,
}

impl EventDispatcher {
    pub fn new() -> Self {
        Self {
            input_handlers: Vec::new(),
            buffer_handlers: Vec::new(),
            ui_handlers: Vec::new(),
            window_handlers: Vec::new(),
            config_handlers: Vec::new(),
            search_handlers: Vec::new(),
            system_handlers: Vec::new(),
        }
    }

    /// Register handlers by event type for better performance
    pub fn register_input_handler(&mut self, handler: Box<dyn EventHandler + Send>) {
        self.input_handlers.push(handler);
    }

    pub fn register_buffer_handler(&mut self, handler: Box<dyn EventHandler + Send>) {
        self.buffer_handlers.push(handler);
    }

    pub fn register_ui_handler(&mut self, handler: Box<dyn EventHandler + Send>) {
        self.ui_handlers.push(handler);
    }

    pub fn register_window_handler(&mut self, handler: Box<dyn EventHandler + Send>) {
        self.window_handlers.push(handler);
    }

    pub fn register_config_handler(&mut self, handler: Box<dyn EventHandler + Send>) {
        self.config_handlers.push(handler);
    }

    pub fn register_search_handler(&mut self, handler: Box<dyn EventHandler + Send>) {
        self.search_handlers.push(handler);
    }

    pub fn register_system_handler(&mut self, handler: Box<dyn EventHandler + Send>) {
        self.system_handlers.push(handler);
    }

    /// Dispatch event to appropriate handlers
    pub fn dispatch(&mut self, event: &EditorEvent) -> Vec<EditorEvent> {
        let handlers = match event {
            EditorEvent::Input(_) => &mut self.input_handlers,
            EditorEvent::Buffer(_) => &mut self.buffer_handlers,
            EditorEvent::UI(_) => &mut self.ui_handlers,
            EditorEvent::Window(_) => &mut self.window_handlers,
            EditorEvent::Config(_) => &mut self.config_handlers,
            EditorEvent::Search(_) => &mut self.search_handlers,
            EditorEvent::System(_) => &mut self.system_handlers,
            EditorEvent::Plugin(_) => return Vec::new(), // TODO: implement plugin handlers
            EditorEvent::LSP(_) => return Vec::new(),    // TODO: implement LSP handlers
        };

        let mut generated_events = Vec::new();
        for handler in handlers {
            if handler.can_handle(event) {
                match handler.handle_event(event) {
                    EventResult::Handled => break,
                    EventResult::PassThrough => continue,
                    EventResult::NotHandled => continue,
                    EventResult::Error(err) => {
                        eprintln!("Event handler error: {}", err);
                        continue;
                    }
                    EventResult::Events(mut events) => {
                        generated_events.append(&mut events);
                        continue;
                    }
                }
            }
        }

        generated_events
    }
}

/// Convenience macros for creating events
#[macro_export]
macro_rules! event {
    (input::key_press($key:expr)) => {
        EditorEvent::Input(InputEvent::KeyPress($key))
    };
    (buffer::modified($id:expr)) => {
        EditorEvent::Buffer(BufferEvent::Modified { buffer_id: $id })
    };
    (ui::redraw) => {
        EditorEvent::UI(UIEvent::RedrawRequest)
    };
    (system::quit) => {
        EditorEvent::System(SystemEvent::Quit)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestHandler;
    impl EventHandler for TestHandler {
        fn handle_event(&mut self, _event: &EditorEvent) -> EventResult {
            EventResult::Handled
        }

        fn can_handle(&self, event: &EditorEvent) -> bool {
            matches!(event, EditorEvent::Input(_))
        }
    }

    #[test]
    fn test_event_bus_creation() {
        let bus = EventBus::new();
        assert_eq!(bus.handlers.len(), 0);
    }

    #[test]
    fn test_event_handler_registration() {
        let mut bus = EventBus::new();
        bus.register_handler(Box::new(TestHandler));
        assert_eq!(bus.handlers.len(), 1);
    }

    #[test]
    fn test_event_macros() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

        let key_event = KeyEvent {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };

        let _event = event!(input::key_press(key_event));
        let _event = event!(buffer::modified(1));
        let _event = event!(ui::redraw);
        let _event = event!(system::quit);
    }
}
