use crate::buffer::Buffer;
use crate::config::EditorConfig;
use crate::keymap::KeyHandler;
use crate::mode::Mode;
use crate::search::{SearchEngine, SearchResult};
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
    /// Editor configuration
    config: EditorConfig,
    /// Search engine for text search
    search_engine: SearchEngine,
    /// Current search results
    search_results: Vec<SearchResult>,
    /// Current search result index
    current_search_index: Option<usize>,
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
        let config = EditorConfig::load();

        // Initialize UI with config values
        let mut ui = UI::new();
        ui.show_line_numbers = config.display.show_line_numbers;
        ui.show_relative_numbers = config.display.show_relative_numbers;
        ui.show_cursor_line = config.display.show_cursor_line;

        let key_handler = KeyHandler::new();

        Ok(Self {
            buffers: HashMap::new(),
            current_buffer_id: None,
            next_buffer_id: 1,
            mode: Mode::Normal,
            terminal,
            ui,
            key_handler,
            config,
            search_engine: SearchEngine::new(),
            search_results: Vec::new(),
            current_search_index: None,
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
                    // Temporarily extract key_handler to avoid borrowing conflicts
                    let mut key_handler =
                        std::mem::replace(&mut self.key_handler, KeyHandler::new());
                    let result = key_handler.handle_key(self, key_event);
                    self.key_handler = key_handler;
                    result?;
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

    /// Perform a search in the current buffer
    pub fn search(&mut self, pattern: &str) -> bool {
        let lines = if let Some(buffer) = self.current_buffer() {
            buffer.lines.clone()
        } else {
            return false;
        };

        let search_results = self.search_engine.search(pattern, &lines);
        self.search_results = search_results;

        if !self.search_results.is_empty() {
            self.current_search_index = Some(0);
            self.move_to_search_result(0);
            self.status_message = format!("Found {} matches", self.search_results.len());
            true
        } else {
            self.current_search_index = None;
            self.status_message = format!("Pattern not found: {}", pattern);
            false
        }
    }

    /// Move to the next search result
    pub fn search_next(&mut self) -> bool {
        if self.search_results.is_empty() {
            self.status_message = "No search results".to_string();
            return false;
        }

        if let Some(current_index) = self.current_search_index {
            let next_index = (current_index + 1) % self.search_results.len();
            self.current_search_index = Some(next_index);
            self.move_to_search_result(next_index);
            self.status_message =
                format!("Match {} of {}", next_index + 1, self.search_results.len());
            true
        } else {
            self.current_search_index = Some(0);
            self.move_to_search_result(0);
            true
        }
    }

    /// Move to the previous search result
    pub fn search_previous(&mut self) -> bool {
        if self.search_results.is_empty() {
            self.status_message = "No search results".to_string();
            return false;
        }

        if let Some(current_index) = self.current_search_index {
            let prev_index = if current_index == 0 {
                self.search_results.len() - 1
            } else {
                current_index - 1
            };
            self.current_search_index = Some(prev_index);
            self.move_to_search_result(prev_index);
            self.status_message =
                format!("Match {} of {}", prev_index + 1, self.search_results.len());
            true
        } else {
            self.current_search_index = Some(0);
            self.move_to_search_result(0);
            true
        }
    }

    /// Move cursor to a specific search result
    fn move_to_search_result(&mut self, index: usize) {
        if let Some(result) = self.search_results.get(index).cloned() {
            if let Some(buffer) = self.current_buffer_mut() {
                buffer.cursor.row = result.line;
                buffer.cursor.col = result.start_col;
            }
        }
    }

    /// Clear current search results
    pub fn clear_search(&mut self) {
        self.search_results.clear();
        self.current_search_index = None;
    }

    /// Toggle absolute line numbers
    pub fn toggle_line_numbers(&mut self) {
        self.ui.show_line_numbers = !self.ui.show_line_numbers;
        let status = if self.ui.show_line_numbers {
            "Line numbers enabled"
        } else {
            "Line numbers disabled"
        };
        self.status_message = status.to_string();
    }

    /// Toggle relative line numbers
    pub fn toggle_relative_numbers(&mut self) {
        self.ui.show_relative_numbers = !self.ui.show_relative_numbers;
        let status = if self.ui.show_relative_numbers {
            "Relative line numbers enabled"
        } else {
            "Relative line numbers disabled"
        };
        self.status_message = status.to_string();
    }

    /// Set line number display options
    pub fn set_line_numbers(&mut self, absolute: bool, relative: bool) {
        self.config.display.show_line_numbers = absolute;
        self.config.display.show_relative_numbers = relative;

        // Update UI to reflect config changes
        self.ui.show_line_numbers = absolute;
        self.ui.show_relative_numbers = relative;

        let status = match (absolute, relative) {
            (true, true) => "Hybrid line numbers enabled",
            (true, false) => "Absolute line numbers enabled",
            (false, true) => "Relative line numbers enabled",
            (false, false) => "Line numbers disabled",
        };
        self.status_message = status.to_string();

        // Save config changes
        let _ = self.config.save();
    }

    /// Toggle cursor line highlighting
    pub fn toggle_cursor_line(&mut self) {
        self.config.display.show_cursor_line = !self.config.display.show_cursor_line;
        self.ui.show_cursor_line = self.config.display.show_cursor_line;
        let status = if self.config.display.show_cursor_line {
            "Cursor line highlighting enabled"
        } else {
            "Cursor line highlighting disabled"
        };
        self.status_message = status.to_string();

        // Save config changes
        let _ = self.config.save();
    }

    /// Set cursor line highlighting
    pub fn set_cursor_line(&mut self, enabled: bool) {
        self.config.display.show_cursor_line = enabled;
        self.ui.show_cursor_line = enabled;

        let status = if enabled {
            "Cursor line highlighting enabled"
        } else {
            "Cursor line highlighting disabled"
        };
        self.status_message = status.to_string();

        // Save config changes
        let _ = self.config.save();
    }

    /// Set a configuration setting by name
    pub fn set_config_setting(&mut self, setting: &str, value: &str) {
        let _ = self.config.set_setting(setting, value);

        // Update UI to reflect config changes
        self.ui.show_line_numbers = self.config.display.show_line_numbers;
        self.ui.show_relative_numbers = self.config.display.show_relative_numbers;
        self.ui.show_cursor_line = self.config.display.show_cursor_line;

        // Save config changes
        let _ = self.config.save();
    }

    /// Get the current value of a configuration setting
    pub fn get_line_number_state(&self) -> (bool, bool) {
        (
            self.config.display.show_line_numbers,
            self.config.display.show_relative_numbers,
        )
    }
}
