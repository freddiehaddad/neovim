use crate::buffer::Buffer;
use crate::config::EditorConfig;
use crate::config_watcher::{ConfigChangeEvent, ConfigWatcher};
use crate::keymap::KeyHandler;
use crate::mode::Mode;
use crate::search::{SearchEngine, SearchResult};
use crate::syntax::{HighlightRange, SyntaxHighlighter};
use crate::terminal::Terminal;
use crate::theme_watcher::ThemeManager;
use crate::ui::UI;
use crate::window::{SplitDirection, WindowManager};
use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

// Struct to hold editor state for rendering without borrowing issues
pub struct EditorRenderState {
    pub mode: Mode,
    pub current_buffer: Option<Buffer>,
    pub all_buffers: HashMap<usize, Buffer>,
    pub command_line: String,
    pub status_message: String,
    pub buffer_count: usize,
    pub current_buffer_id: Option<usize>,
    pub current_window_id: Option<usize>,
    pub window_manager: WindowManager,
    pub syntax_highlights: HashMap<usize, Vec<HighlightRange>>, // line_index -> highlights
}

pub struct Editor {
    /// All open buffers
    buffers: HashMap<usize, Buffer>,
    /// Currently active buffer ID
    current_buffer_id: Option<usize>,
    /// Next buffer ID to assign
    next_buffer_id: usize,
    /// Window management for splits
    window_manager: WindowManager,
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
    /// Configuration file watcher for hot reloading
    config_watcher: Option<ConfigWatcher>,
    /// Theme manager for hot reloading themes
    theme_manager: ThemeManager,
    /// Syntax highlighter for code highlighting
    syntax_highlighter: Option<SyntaxHighlighter>,
}

impl Editor {
    pub fn new() -> Result<Self> {
        let terminal = Terminal::new()?;
        let config = EditorConfig::load();

        // Get terminal size for window manager
        let (terminal_width, terminal_height) = terminal.size();

        // Initialize UI with config values
        let mut ui = UI::new();
        ui.show_line_numbers = config.display.show_line_numbers;
        ui.show_relative_numbers = config.display.show_relative_numbers;
        ui.show_cursor_line = config.display.show_cursor_line;
        ui.set_theme(&config.display.color_scheme);

        let key_handler = KeyHandler::new();

        // Initialize window manager
        let window_manager = WindowManager::new(terminal_width, terminal_height);

        // Initialize config watcher for hot reloading
        let config_watcher = ConfigWatcher::new().ok(); // Don't fail if watcher can't be created

        // Initialize theme manager for hot reloading themes
        let theme_manager = ThemeManager::new();

        // Initialize syntax highlighter
        let syntax_highlighter = SyntaxHighlighter::new().ok(); // Don't fail if highlighter can't be created

        Ok(Self {
            buffers: HashMap::new(),
            current_buffer_id: None,
            next_buffer_id: 1,
            window_manager,
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
            config_watcher,
            theme_manager,
            syntax_highlighter,
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

        // Assign buffer to current window
        if let Some(current_window) = self.window_manager.current_window_mut() {
            current_window.set_buffer(id);
            // Initialize window cursor position from buffer's cursor position
            if let Some(buffer) = self.buffers.get(&id) {
                current_window.save_cursor_position(buffer.cursor.row, buffer.cursor.col);
            }
        }

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

    /// Open a file in a new buffer
    pub fn open_file(&mut self, filename: &str) -> Result<String> {
        let path = PathBuf::from(filename);
        let buffer_id = self.create_buffer(Some(path))?;
        Ok(format!("Opened '{}' in buffer {}", filename, buffer_id))
    }

    /// Switch to the next buffer in the list
    pub fn switch_to_next_buffer(&mut self) -> bool {
        if self.buffers.len() <= 1 {
            return false;
        }

        let buffer_ids: Vec<usize> = self.buffers.keys().copied().collect();
        let current_index = buffer_ids
            .iter()
            .position(|&id| Some(id) == self.current_buffer_id)
            .unwrap_or(0);

        let next_index = (current_index + 1) % buffer_ids.len();
        self.current_buffer_id = Some(buffer_ids[next_index]);
        true
    }

    /// Switch to the previous buffer in the list
    pub fn switch_to_previous_buffer(&mut self) -> bool {
        if self.buffers.len() <= 1 {
            return false;
        }

        let buffer_ids: Vec<usize> = self.buffers.keys().copied().collect();
        let current_index = buffer_ids
            .iter()
            .position(|&id| Some(id) == self.current_buffer_id)
            .unwrap_or(0);

        let prev_index = if current_index == 0 {
            buffer_ids.len() - 1
        } else {
            current_index - 1
        };
        self.current_buffer_id = Some(buffer_ids[prev_index]);
        true
    }

    /// Close the current buffer
    pub fn close_current_buffer(&mut self) -> Result<String> {
        if let Some(current_id) = self.current_buffer_id {
            if let Some(buffer) = self.buffers.get(&current_id) {
                if buffer.modified {
                    return Ok("Buffer has unsaved changes! Use :bd! to force close".to_string());
                }
            }

            self.buffers.remove(&current_id);

            // Switch to another buffer or create a new one if this was the last
            if self.buffers.is_empty() {
                self.create_buffer(None)?;
                Ok("Closed buffer, created new empty buffer".to_string())
            } else {
                self.current_buffer_id = self.buffers.keys().next().copied();
                Ok("Buffer closed".to_string())
            }
        } else {
            Ok("No buffer to close".to_string())
        }
    }

    /// Force close the current buffer (ignore unsaved changes)
    pub fn force_close_current_buffer(&mut self) -> Result<String> {
        if let Some(current_id) = self.current_buffer_id {
            self.buffers.remove(&current_id);

            // Switch to another buffer or create a new one if this was the last
            if self.buffers.is_empty() {
                self.create_buffer(None)?;
                Ok("Closed buffer (discarded changes), created new empty buffer".to_string())
            } else {
                self.current_buffer_id = self.buffers.keys().next().copied();
                Ok("Buffer closed (discarded changes)".to_string())
            }
        } else {
            Ok("No buffer to close".to_string())
        }
    }

    /// Switch to buffer by name (partial matching)
    pub fn switch_to_buffer_by_name(&mut self, name: &str) -> bool {
        for (id, buffer) in &self.buffers {
            if let Some(file_path) = &buffer.file_path {
                if let Some(filename) = file_path.file_name() {
                    if filename.to_string_lossy().contains(name) {
                        self.current_buffer_id = Some(*id);
                        return true;
                    }
                }
            }
        }
        false
    }

    /// List all open buffers
    pub fn list_buffers(&self) -> String {
        if self.buffers.is_empty() {
            return "No buffers open".to_string();
        }

        let mut buffer_list = String::from("Buffers: ");
        for (id, buffer) in &self.buffers {
            let is_current = Some(*id) == self.current_buffer_id;
            let modified = if buffer.modified { "+" } else { "" };
            let name = buffer
                .file_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "[No Name]".to_string());

            buffer_list.push_str(&format!(
                "{}{}:{}{}{}",
                if is_current { "[" } else { "" },
                id,
                name,
                modified,
                if is_current { "]" } else { "" }
            ));

            buffer_list.push(' ');
        }

        buffer_list.trim_end().to_string()
    }

    fn render(&mut self) -> Result<()> {
        // Collect all needed data first
        let mode = self.mode;
        let current_buffer = self.current_buffer().cloned();
        let command_line = self.command_line.clone();
        let status_message = self.status_message.clone();

        // Update window viewport based on cursor position
        if let Some(buffer) = &current_buffer {
            if let Some(current_window) = self.window_manager.current_window_mut() {
                let content_height = current_window.content_height();
                let cursor_row = buffer.cursor.row;

                // Check if cursor is outside current viewport
                let viewport_bottom = current_window.viewport_top + content_height;

                if cursor_row < current_window.viewport_top {
                    // Cursor moved above viewport - scroll up
                    current_window.viewport_top = cursor_row;
                } else if cursor_row >= viewport_bottom {
                    // Cursor moved below viewport - scroll down
                    current_window.viewport_top = cursor_row.saturating_sub(content_height - 1);
                }
            }
        }

        // Generate syntax highlights for visible lines only
        let mut syntax_highlights = HashMap::new();
        if let Some(buffer) = &current_buffer {
            // Get the visible range from current window
            if let Some(current_window) = self.window_manager.current_window() {
                let content_height = current_window.content_height();
                let viewport_top = current_window.viewport_top;

                // Only highlight visible lines + a small buffer for smooth scrolling
                let highlight_start = viewport_top;
                let highlight_end =
                    std::cmp::min(viewport_top + content_height + 10, buffer.lines.len()); // 10 line buffer

                for line_index in highlight_start..highlight_end {
                    if let Some(line) = buffer.get_line(line_index) {
                        let file_path = buffer
                            .file_path
                            .as_ref()
                            .map(|p| p.to_string_lossy().to_string());
                        if let Some(path) = file_path {
                            let highlights = self.get_syntax_highlights(line, Some(&path));
                            if !highlights.is_empty() {
                                syntax_highlights.insert(line_index, highlights);
                            }
                        }
                    }
                }
            }
        }

        // Create a temporary editor state for rendering
        let editor_state = EditorRenderState {
            mode,
            current_buffer,
            all_buffers: self.buffers.clone(),
            command_line,
            status_message,
            buffer_count: self.buffers.len(),
            current_buffer_id: self.current_buffer_id,
            current_window_id: self.window_manager.current_window_id(),
            window_manager: self.window_manager.clone(),
            syntax_highlights,
        };

        // Now we can safely borrow terminal mutably
        self.ui.render(&mut self.terminal, &editor_state)?;
        self.terminal.flush()?;
        Ok(())
    }

    fn handle_input(&mut self) -> Result<bool> {
        let mut input_processed = false;

        // Check for config file changes first
        if let Some(ref watcher) = self.config_watcher {
            let changes = watcher.check_for_changes();
            for change in changes {
                match change {
                    ConfigChangeEvent::EditorConfigChanged => {
                        self.reload_editor_config();
                        input_processed = true;
                    }
                    ConfigChangeEvent::KeymapConfigChanged => {
                        self.reload_keymap_config();
                        input_processed = true;
                    }
                }
            }
        }

        // Check for theme file changes
        if let Ok(theme_changed) = self.theme_manager.check_and_reload() {
            if theme_changed {
                self.reload_ui_theme();
                input_processed = true;
            }
        }

        // Handle keyboard input
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key_event) = event::read()? {
                // Only process key press events, not release events
                if key_event.kind == KeyEventKind::Press {
                    // Temporarily take the KeyHandler to avoid borrowing conflicts
                    let mut key_handler = std::mem::take(&mut self.key_handler);
                    let result = key_handler.handle_key(self, key_event);
                    self.key_handler = key_handler;
                    result?;

                    // Sync the current buffer's cursor position to the current window
                    self.sync_cursor_to_current_window();

                    input_processed = true;
                }
            } else if let Event::Resize(width, height) = event::read()? {
                // Handle terminal resize
                self.window_manager.resize_terminal(width, height);
                input_processed = true;
            }
        }

        Ok(input_processed)
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
        self.ui.set_theme(&self.config.display.color_scheme);

        // Apply specific settings that need immediate effect
        match setting {
            "syntax" | "syn" => {
                if self.config.display.syntax_highlighting {
                    // Re-enable syntax highlighting
                    if self.syntax_highlighter.is_none() {
                        self.syntax_highlighter = SyntaxHighlighter::new().ok();
                    }
                } else {
                    // Disable syntax highlighting
                    self.syntax_highlighter = None;
                }
            }
            "colorscheme" | "colo" => {
                // Update theme manager to match the new colorscheme
                let _ = self.theme_manager.set_current_theme(value);
                self.ui.set_theme(value);

                // Update syntax highlighter with new color scheme
                if let Some(ref mut highlighter) = self.syntax_highlighter {
                    if let Err(e) = highlighter.update_theme(&self.config.display.color_scheme) {
                        self.set_status_message(format!("Failed to load color scheme: {}", e));
                    }
                }
            }
            "ignorecase" | "ic" | "smartcase" | "scs" => {
                self.apply_search_settings();
            }
            "autosave" | "aw" => {
                // Auto save setting changed, check if we should save now
                self.check_auto_save();
            }
            _ => {}
        }

        // Save config changes
        let _ = self.config.save();
    }

    /// Apply tab settings to current buffer
    pub fn apply_tab_settings(&mut self) {
        // Tab settings are handled at the editor level since Buffer doesn't store them
        // These settings affect how input is processed
    }

    /// Apply search settings
    pub fn apply_search_settings(&mut self) {
        // Update search engine case sensitivity
        self.search_engine
            .set_case_sensitive(!self.config.behavior.ignore_case);
        // Smart case logic would be implemented in search methods
    }

    /// Check if auto save is enabled and save if needed
    pub fn check_auto_save(&mut self) {
        if self.config.editing.auto_save {
            if let Some(buffer) = self.current_buffer_mut() {
                if buffer.modified && buffer.file_path.is_some() {
                    let _ = buffer.save();
                }
            }
        }
    }

    /// Get configuration value for display  
    pub fn get_config_value(&self, setting: &str) -> Option<String> {
        match setting {
            "number" | "nu" => Some(self.config.display.show_line_numbers.to_string()),
            "relativenumber" | "rnu" => Some(self.config.display.show_relative_numbers.to_string()),
            "cursorline" | "cul" => Some(self.config.display.show_cursor_line.to_string()),
            "tabstop" | "ts" => Some(self.config.behavior.tab_width.to_string()),
            "expandtab" | "et" => Some(self.config.behavior.expand_tabs.to_string()),
            "autoindent" | "ai" => Some(self.config.behavior.auto_indent.to_string()),
            "ignorecase" | "ic" => Some(self.config.behavior.ignore_case.to_string()),
            "smartcase" | "scs" => Some(self.config.behavior.smart_case.to_string()),
            "hlsearch" | "hls" => Some(self.config.behavior.highlight_search.to_string()),
            "incsearch" | "is" => Some(self.config.behavior.incremental_search.to_string()),
            "wrap" => Some(self.config.behavior.wrap_lines.to_string()),
            "linebreak" | "lbr" => Some(self.config.behavior.line_break.to_string()),
            "undolevels" | "ul" => Some(self.config.editing.undo_levels.to_string()),
            "undofile" | "udf" => Some(self.config.editing.persistent_undo.to_string()),
            "backup" | "bk" => Some(self.config.editing.backup.to_string()),
            "swapfile" | "swf" => Some(self.config.editing.swap_file.to_string()),
            "autosave" | "aw" => Some(self.config.editing.auto_save.to_string()),
            "laststatus" | "ls" => Some(self.config.interface.show_status_line.to_string()),
            "showcmd" | "sc" => Some(self.config.interface.show_command.to_string()),
            "scrolloff" | "so" => Some(self.config.interface.scroll_off.to_string()),
            "sidescrolloff" | "siso" => Some(self.config.interface.side_scroll_off.to_string()),
            "timeoutlen" | "tm" => Some(self.config.interface.command_timeout.to_string()),
            "colorscheme" | "colo" => Some(self.config.display.color_scheme.clone()),
            "syntax" | "syn" => Some(self.config.display.syntax_highlighting.to_string()),
            _ => None,
        }
    }

    /// Get the current value of a configuration setting
    pub fn get_line_number_state(&self) -> (bool, bool) {
        (
            self.config.display.show_line_numbers,
            self.config.display.show_relative_numbers,
        )
    }

    /// Get syntax highlights for a line of text
    pub fn get_syntax_highlights(
        &mut self,
        text: &str,
        file_path: Option<&str>,
    ) -> Vec<crate::syntax::HighlightRange> {
        if let (Some(highlighter), Some(path)) = (&mut self.syntax_highlighter, file_path) {
            if let Some(language) = highlighter.detect_language_from_extension(path) {
                highlighter
                    .highlight_text(text, &language)
                    .unwrap_or_default()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    }

    /// Get highlighted text for a specific line in the current buffer
    pub fn get_line_highlights(&mut self, line_index: usize) -> Vec<crate::syntax::HighlightRange> {
        // Get the necessary data first to avoid borrow conflicts
        let (line_text, file_path) = {
            if let Some(buffer) = self.current_buffer() {
                let line = buffer.get_line(line_index).map(|s| s.to_string());
                let path = buffer
                    .file_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string());
                (line, path)
            } else {
                (None, None)
            }
        };

        if let (Some(line), Some(path)) = (line_text, file_path) {
            self.get_syntax_highlights(&line, Some(&path))
        } else {
            Vec::new()
        }
    }

    /// Reload editor configuration from editor.toml
    fn reload_editor_config(&mut self) {
        let new_config = EditorConfig::load();

        // Update UI to reflect new config values
        self.ui.show_line_numbers = new_config.display.show_line_numbers;
        self.ui.show_relative_numbers = new_config.display.show_relative_numbers;
        self.ui.show_cursor_line = new_config.display.show_cursor_line;

        self.config = new_config;
        self.status_message = "Editor configuration reloaded".to_string();
    }

    /// Reload keymap configuration from keymaps.toml
    fn reload_keymap_config(&mut self) {
        self.key_handler = KeyHandler::new(); // This will reload the keymaps.toml
        self.status_message = "Keymap configuration reloaded".to_string();
    }

    /// Reload UI theme from themes.toml
    fn reload_ui_theme(&mut self) {
        // Get the current theme name from the theme manager
        let current_theme = self.theme_manager.current_theme_name();

        // Update the UI with the new theme
        self.ui.set_theme(current_theme);

        self.status_message = format!("Theme '{}' reloaded", current_theme);
    }

    // Scrolling methods
    pub fn scroll_down_line(&mut self) {
        // Ctrl+e: Scroll down one line, cursor stays at same screen position
        let old_viewport_top = self.ui.viewport_top();
        self.ui.scroll_down_line();
        let new_viewport_top = self.ui.viewport_top();

        // Adjust cursor to maintain screen position
        if let Some(buffer) = self.current_buffer_mut() {
            if old_viewport_top != new_viewport_top {
                buffer.cursor.row = buffer
                    .cursor
                    .row
                    .saturating_add(new_viewport_top - old_viewport_top);
                buffer.cursor.row = buffer.cursor.row.min(buffer.lines.len().saturating_sub(1));
            }
        }
    }

    pub fn scroll_up_line(&mut self) {
        // Ctrl+y: Scroll up one line, cursor stays at same screen position
        let old_viewport_top = self.ui.viewport_top();
        self.ui.scroll_up_line();
        let new_viewport_top = self.ui.viewport_top();

        // Adjust cursor to maintain screen position
        if let Some(buffer) = self.current_buffer_mut() {
            if old_viewport_top != new_viewport_top {
                buffer.cursor.row = buffer
                    .cursor
                    .row
                    .saturating_sub(old_viewport_top - new_viewport_top);
            }
        }
    }

    pub fn scroll_down_page(&mut self) {
        // Ctrl+f: Scroll down one page, cursor moves with viewport
        let old_viewport_top = self.ui.viewport_top();
        self.ui.scroll_down_page();
        let new_viewport_top = self.ui.viewport_top();

        // Move cursor down by the same amount as viewport
        if let Some(buffer) = self.current_buffer_mut() {
            if old_viewport_top != new_viewport_top {
                let scroll_amount = new_viewport_top - old_viewport_top;
                buffer.cursor.row = buffer.cursor.row.saturating_add(scroll_amount);
                buffer.cursor.row = buffer.cursor.row.min(buffer.lines.len().saturating_sub(1));

                // Ensure cursor column is valid for the new line
                if let Some(line) = buffer.get_line(buffer.cursor.row) {
                    buffer.cursor.col = buffer.cursor.col.min(line.len());
                }
            }
        }
    }

    pub fn scroll_up_page(&mut self) {
        // Ctrl+b: Scroll up one page, cursor moves with viewport
        let old_viewport_top = self.ui.viewport_top();
        self.ui.scroll_up_page();
        let new_viewport_top = self.ui.viewport_top();

        // Move cursor up by the same amount as viewport
        if let Some(buffer) = self.current_buffer_mut() {
            if old_viewport_top != new_viewport_top {
                let scroll_amount = old_viewport_top - new_viewport_top;
                buffer.cursor.row = buffer.cursor.row.saturating_sub(scroll_amount);

                // Ensure cursor column is valid for the new line
                if let Some(line) = buffer.get_line(buffer.cursor.row) {
                    buffer.cursor.col = buffer.cursor.col.min(line.len());
                }
            }
        }
    }

    pub fn scroll_down_half_page(&mut self) {
        // Ctrl+d: Scroll down half page, cursor moves with viewport
        let old_viewport_top = self.ui.viewport_top();
        self.ui.scroll_down_half_page();
        let new_viewport_top = self.ui.viewport_top();

        // Move cursor down by the same amount as viewport
        if let Some(buffer) = self.current_buffer_mut() {
            if old_viewport_top != new_viewport_top {
                let scroll_amount = new_viewport_top - old_viewport_top;
                buffer.cursor.row = buffer.cursor.row.saturating_add(scroll_amount);
                buffer.cursor.row = buffer.cursor.row.min(buffer.lines.len().saturating_sub(1));

                // Ensure cursor column is valid for the new line
                if let Some(line) = buffer.get_line(buffer.cursor.row) {
                    buffer.cursor.col = buffer.cursor.col.min(line.len());
                }
            }
        }
    }

    pub fn scroll_up_half_page(&mut self) {
        // Ctrl+u: Scroll up half page, cursor moves with viewport
        let old_viewport_top = self.ui.viewport_top();
        self.ui.scroll_up_half_page();
        let new_viewport_top = self.ui.viewport_top();

        // Move cursor up by the same amount as viewport
        if let Some(buffer) = self.current_buffer_mut() {
            if old_viewport_top != new_viewport_top {
                let scroll_amount = old_viewport_top - new_viewport_top;
                buffer.cursor.row = buffer.cursor.row.saturating_sub(scroll_amount);

                // Ensure cursor column is valid for the new line
                if let Some(line) = buffer.get_line(buffer.cursor.row) {
                    buffer.cursor.col = buffer.cursor.col.min(line.len());
                }
            }
        }
    }

    // Centering methods (z commands in Vim)
    pub fn center_cursor(&mut self) {
        // zz: Center current line in viewport
        if let Some(buffer) = self.current_buffer() {
            let cursor_row = buffer.cursor.row;
            let buffer_lines_len = buffer.lines.len();
            let (_, height) = self.terminal.size();

            self.ui.center_cursor(cursor_row, height);

            // Ensure we don't scroll past the end of the buffer
            let content_height = height.saturating_sub(2) as usize;
            let max_viewport_top = buffer_lines_len.saturating_sub(content_height);
            let current_viewport = self.ui.viewport_top().min(max_viewport_top);
            self.ui.set_viewport_top(current_viewport);
        }
    }

    pub fn cursor_to_top(&mut self) {
        // zt: Move current line to top of viewport
        if let Some(buffer) = self.current_buffer() {
            let cursor_row = buffer.cursor.row;
            self.ui.cursor_to_top(cursor_row);
        }
    }

    pub fn cursor_to_bottom(&mut self) {
        // zb: Move current line to bottom of viewport
        if let Some(buffer) = self.current_buffer() {
            let cursor_row = buffer.cursor.row;
            let (_, height) = self.terminal.size();
            self.ui.cursor_to_bottom(cursor_row, height);
        }
    }

    /// Helper method to set up a new window with buffer and cursor position
    fn setup_new_window(&mut self, new_window_id: usize) {
        if let Some(buffer_id) = self.current_buffer_id {
            if let Some(buffer) = self.buffers.get(&buffer_id) {
                if let Some(new_window) = self.window_manager.get_window_mut(new_window_id) {
                    new_window.set_buffer(buffer_id);
                    // Copy current cursor position to the new window
                    new_window.save_cursor_position(buffer.cursor.row, buffer.cursor.col);
                }
            }
        }
    }

    // Split window methods
    pub fn split_horizontal(&mut self) -> String {
        if let Some(new_window_id) = self
            .window_manager
            .split_current_window(SplitDirection::Horizontal)
        {
            // Set up the new window with buffer and cursor position
            self.setup_new_window(new_window_id);
            format!("Created horizontal split (window {})", new_window_id)
        } else {
            "Failed to create horizontal split".to_string()
        }
    }

    pub fn split_vertical(&mut self) -> String {
        if let Some(new_window_id) = self
            .window_manager
            .split_current_window(SplitDirection::Vertical)
        {
            // Set up the new window with buffer and cursor position
            self.setup_new_window(new_window_id);
            format!("Created vertical split (window {})", new_window_id)
        } else {
            "Failed to create vertical split".to_string()
        }
    }

    pub fn split_horizontal_above(&mut self) -> String {
        if let Some(new_window_id) = self
            .window_manager
            .split_current_window(SplitDirection::HorizontalAbove)
        {
            // Set up the new window with buffer and cursor position
            self.setup_new_window(new_window_id);
            format!("Created horizontal split above (window {})", new_window_id)
        } else {
            "Failed to create horizontal split above".to_string()
        }
    }

    pub fn split_horizontal_below(&mut self) -> String {
        if let Some(new_window_id) = self
            .window_manager
            .split_current_window(SplitDirection::HorizontalBelow)
        {
            // Set up the new window with buffer and cursor position
            self.setup_new_window(new_window_id);
            format!("Created horizontal split below (window {})", new_window_id)
        } else {
            "Failed to create horizontal split below".to_string()
        }
    }

    pub fn split_vertical_left(&mut self) -> String {
        if let Some(new_window_id) = self
            .window_manager
            .split_current_window(SplitDirection::VerticalLeft)
        {
            // Set up the new window with buffer and cursor position
            self.setup_new_window(new_window_id);
            format!("Created vertical split left (window {})", new_window_id)
        } else {
            "Failed to create vertical split left".to_string()
        }
    }

    pub fn split_vertical_right(&mut self) -> String {
        if let Some(new_window_id) = self
            .window_manager
            .split_current_window(SplitDirection::VerticalRight)
        {
            // Set up the new window with buffer and cursor position
            self.setup_new_window(new_window_id);
            format!("Created vertical split right (window {})", new_window_id)
        } else {
            "Failed to create vertical split right".to_string()
        }
    }

    pub fn close_window(&mut self) -> String {
        if self.window_manager.close_current_window() {
            // Update current buffer based on new current window
            if let Some(current_window) = self.window_manager.current_window() {
                self.current_buffer_id = current_window.buffer_id;
            }
            "Window closed".to_string()
        } else {
            "Cannot close the last window".to_string()
        }
    }

    // Window navigation methods
    pub fn move_to_window_left(&mut self) -> bool {
        // Save current cursor position before switching
        self.save_current_cursor_to_window();

        let result = self.window_manager.move_to_window_left();
        if result {
            self.restore_cursor_from_current_window();
        }
        result
    }

    pub fn move_to_window_right(&mut self) -> bool {
        // Save current cursor position before switching
        self.save_current_cursor_to_window();

        let result = self.window_manager.move_to_window_right();
        if result {
            self.restore_cursor_from_current_window();
        }
        result
    }

    pub fn move_to_window_up(&mut self) -> bool {
        // Save current cursor position before switching
        self.save_current_cursor_to_window();

        let result = self.window_manager.move_to_window_up();
        if result {
            self.restore_cursor_from_current_window();
        }
        result
    }

    pub fn move_to_window_down(&mut self) -> bool {
        // Save current cursor position before switching
        self.save_current_cursor_to_window();

        let result = self.window_manager.move_to_window_down();
        if result {
            self.restore_cursor_from_current_window();
        }
        result
    }

    fn sync_cursor_to_current_window(&mut self) {
        if let (Some(current_buffer_id), Some(current_window_id)) = (
            self.current_buffer_id,
            self.window_manager.current_window_id(),
        ) {
            if let Some(current_buffer) = self.buffers.get(&current_buffer_id) {
                if let Some(current_window) = self.window_manager.get_window_mut(current_window_id)
                {
                    current_window
                        .save_cursor_position(current_buffer.cursor.row, current_buffer.cursor.col);
                }
            }
        }
    }

    fn save_current_cursor_to_window(&mut self) {
        if let (Some(current_buffer_id), Some(current_window_id)) = (
            self.current_buffer_id,
            self.window_manager.current_window_id(),
        ) {
            if let Some(current_buffer) = self.buffers.get(&current_buffer_id) {
                if let Some(current_window) = self.window_manager.get_window_mut(current_window_id)
                {
                    current_window
                        .save_cursor_position(current_buffer.cursor.row, current_buffer.cursor.col);
                }
            }
        }
    }

    fn restore_cursor_from_current_window(&mut self) {
        // Switch to the new window's buffer
        if let Some(new_window) = self.window_manager.current_window() {
            self.current_buffer_id = new_window.buffer_id;

            // Restore cursor position from the new window
            if let Some(buffer_id) = new_window.buffer_id {
                let (cursor_row, cursor_col) = new_window.get_cursor_position();
                if let Some(buffer) = self.buffers.get_mut(&buffer_id) {
                    buffer.move_cursor(crate::mode::Position::new(cursor_row, cursor_col));
                }
            }
        }
    }
}
