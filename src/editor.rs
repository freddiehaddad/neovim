use crate::buffer::Buffer;
use crate::keymap::KeyHandler;
use crate::mode::Mode;
use crate::terminal::Terminal;
use crate::ui::UI;
use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

// Struct to hold editor state for rendering without borrowing issues
pub struct EditorRenderState {
    pub mode: Mode,
    pub current_buffer: Option<Buffer>,
    pub command_line: String,
    pub status_message: String,
}

pub struct Editor {
    /// All open buffers
    buffers: HashMap<usize, Buffer>,
    /// Currently active buffer ID
    current_buffer_id: Option<usize>,
    /// Next buffer ID to assign
    next_buffer_id: usize,
    /// Current editor mode
    mode: Mode,
    /// Terminal interface
    terminal: Terminal,
    /// UI renderer
    ui: UI,
    /// Key handler for mode-specific input
    key_handler: KeyHandler,
    /// Whether the editor should quit
    should_quit: bool,
    /// Command line content (for command mode)
    command_line: String,
    /// Status message
    status_message: String,
    /// Last render time for frame rate limiting
    last_render_time: Instant,
    /// Minimum time between renders (60 FPS = ~16ms)
    render_interval: Duration,
}

impl Editor {
    pub fn new() -> Result<Self> {
        let terminal = Terminal::new()?;
        let ui = UI::new();
        let key_handler = KeyHandler::new();

        Ok(Self {
            buffers: HashMap::new(),
            current_buffer_id: None,
            next_buffer_id: 1,
            mode: Mode::Normal,
            terminal,
            ui,
            key_handler,
            should_quit: false,
            command_line: String::new(),
            status_message: String::new(),
            last_render_time: Instant::now(),
            render_interval: Duration::from_millis(16), // ~60 FPS
        })
    }

    pub fn run(&mut self) -> Result<()> {
        // Create an initial buffer if none exists
        if self.buffers.is_empty() {
            self.create_buffer(None)?;
        }

        // Initial render
        self.render()?;

        loop {
            if self.should_quit {
                break;
            }

            // Only handle input, render only when needed
            let input_handled = self.handle_input()?;

            // Only re-render if we processed an input event and enough time has passed
            if input_handled {
                let now = Instant::now();
                if now.duration_since(self.last_render_time) >= self.render_interval {
                    self.render()?;
                    self.last_render_time = now;
                }
            }
        }

        Ok(())
    }

    pub fn create_buffer(&mut self, file_path: Option<PathBuf>) -> Result<usize> {
        let id = self.next_buffer_id;
        self.next_buffer_id += 1;

        let buffer = if let Some(path) = file_path {
            Buffer::from_file(id, path)?
        } else {
            Buffer::new(id)
        };

        self.buffers.insert(id, buffer);
        self.current_buffer_id = Some(id);
        Ok(id)
    }

    pub fn current_buffer(&self) -> Option<&Buffer> {
        self.current_buffer_id.and_then(|id| self.buffers.get(&id))
    }

    pub fn current_buffer_mut(&mut self) -> Option<&mut Buffer> {
        self.current_buffer_id
            .and_then(|id| self.buffers.get_mut(&id))
    }

    pub fn switch_to_buffer(&mut self, id: usize) -> bool {
        if self.buffers.contains_key(&id) {
            self.current_buffer_id = Some(id);
            true
        } else {
            false
        }
    }

    pub fn close_buffer(&mut self, id: usize) -> Result<()> {
        if let Some(buffer) = self.buffers.get(&id) {
            if buffer.modified {
                // TODO: Handle unsaved changes
                self.status_message = "Buffer has unsaved changes!".to_string();
                return Ok(());
            }
        }

        self.buffers.remove(&id);

        // Switch to another buffer if we closed the current one
        if self.current_buffer_id == Some(id) {
            self.current_buffer_id = self.buffers.keys().next().copied();
        }

        Ok(())
    }

    fn render(&mut self) -> Result<()> {
        // Collect all needed data first
        let mode = self.mode;
        let current_buffer = self.current_buffer().cloned();
        let command_line = self.command_line.clone();
        let status_message = self.status_message.clone();

        // Create a temporary editor state for rendering
        let editor_state = EditorRenderState {
            mode,
            current_buffer,
            command_line,
            status_message,
        };

        // Now we can safely borrow terminal mutably
        self.ui.render(&mut self.terminal, &editor_state)?;
        self.terminal.flush()?;
        Ok(())
    }

    fn handle_input(&mut self) -> Result<bool> {
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key_event) = event::read()? {
                // Only process key press events, not release events
                if key_event.kind == KeyEventKind::Press {
                    // Clone the key handler to avoid borrowing issues
                    let key_handler = self.key_handler.clone();
                    key_handler.handle_key(self, key_event)?;
                    return Ok(true); // Input was processed
                }
            }
        }
        Ok(false) // No input was processed
    }

    // Getters for UI and other components
    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        if mode != Mode::Command {
            self.command_line.clear();
        }
    }

    pub fn command_line(&self) -> &str {
        &self.command_line
    }

    pub fn set_command_line(&mut self, text: String) {
        self.command_line = text;
    }

    pub fn status_message(&self) -> &str {
        &self.status_message
    }

    pub fn set_status_message(&mut self, message: String) {
        self.status_message = message;
    }

    pub fn quit(&mut self) {
        // Check for unsaved buffers
        let unsaved = self.buffers.values().any(|b| b.modified);
        if unsaved {
            self.status_message = "Unsaved changes! Use :q! to force quit".to_string();
            return;
        }

        self.should_quit = true;
    }

    pub fn force_quit(&mut self) {
        self.should_quit = true;
    }

    pub fn save_current_buffer(&mut self) -> Result<()> {
        if let Some(buffer) = self.current_buffer_mut() {
            buffer.save()?;
            self.status_message = "File saved".to_string();
        }
        Ok(())
    }
}
