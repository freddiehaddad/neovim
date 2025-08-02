use crate::editor::EditorRenderState;
use crate::mode::{Mode, Position};
use crate::syntax::HighlightRange;
use crate::terminal::Terminal;
use crate::theme::{ThemeConfig, UITheme};
use std::io;

pub struct UI {
    /// Top row of the current viewport
    viewport_top: usize,
    /// Show absolute line numbers
    pub show_line_numbers: bool,
    /// Show relative line numbers
    pub show_relative_numbers: bool,
    /// Highlight the current cursor line
    pub show_cursor_line: bool,
    /// Current UI theme from themes.toml
    theme: UITheme,
}

impl UI {
    pub fn new() -> Self {
        // Load theme configuration from themes.toml
        let theme_config = ThemeConfig::load();
        let current_theme = theme_config.get_current_theme();

        Self {
            viewport_top: 0,
            show_line_numbers: true,      // Enable by default like Vim
            show_relative_numbers: false, // Disabled by default
            show_cursor_line: true,       // Enable by default
            theme: current_theme.ui,      // Use theme from themes.toml
        }
    }

    /// Set the UI theme by loading from themes.toml
    pub fn set_theme(&mut self, theme_name: &str) {
        let theme_config = ThemeConfig::load();
        if let Some(complete_theme) = theme_config.get_theme(theme_name) {
            self.theme = complete_theme.ui;
        } else {
            // Fallback to default theme if theme not found
            let default_theme = theme_config.get_current_theme();
            self.theme = default_theme.ui;
        }
    }

    /// Get current theme name
    pub fn theme_name(&self) -> &'static str {
        // Load current theme from config
        let theme_config = ThemeConfig::load();
        // For now, return the current theme name - could be enhanced to track theme state
        if theme_config.theme.current == "dark" {
            "dark"
        } else if theme_config.theme.current == "light" {
            "light"
        } else if theme_config.theme.current == "ferris" {
            "ferris"
        } else {
            "default"
        }
    }

    pub fn render(
        &mut self,
        terminal: &mut Terminal,
        editor_state: &EditorRenderState,
    ) -> io::Result<()> {
        let (width, height) = terminal.size();

        // Start double buffering - queue all operations without immediate display
        terminal.queue_hide_cursor()?;

        // Render all windows
        self.render_windows(terminal, editor_state)?;

        // Render status line
        self.render_status_line(terminal, editor_state, width, height)?;

        // Render command line if in command or search mode
        if editor_state.mode == Mode::Command || editor_state.mode == Mode::Search {
            self.render_command_line(terminal, editor_state, width, height)?;
        }

        // Position cursor and show it
        self.position_cursor(terminal, editor_state)?;

        terminal.queue_show_cursor()?;

        // End double buffering - flush all queued operations at once
        // This eliminates flicker by making all changes appear atomically
        terminal.flush()?;

        Ok(())
    }

    fn render_windows(
        &self,
        terminal: &mut Terminal,
        editor_state: &EditorRenderState,
    ) -> io::Result<()> {
        // Render each window
        for window in editor_state.window_manager.all_windows().values() {
            // Get the buffer for this window
            let buffer = if let Some(buffer_id) = window.buffer_id {
                editor_state.all_buffers.get(&buffer_id)
            } else {
                continue;
            };

            if let Some(buffer) = buffer {
                self.render_window_buffer(terminal, window, buffer, editor_state)?;

                // Draw window border if there are multiple windows
                if editor_state.window_manager.window_count() > 1 {
                    self.render_window_border(
                        terminal,
                        window,
                        editor_state.current_window_id == Some(window.id),
                    )?;
                }
            }
        }
        Ok(())
    }

    fn render_window_buffer(
        &self,
        terminal: &mut Terminal,
        window: &crate::window::Window,
        buffer: &crate::buffer::Buffer,
        editor_state: &EditorRenderState,
    ) -> io::Result<()> {
        let content_height = window.content_height();

        // Calculate line number column width for this window
        let line_number_width = if self.show_line_numbers || self.show_relative_numbers {
            let max_line_num = buffer.lines.len();
            let width = max_line_num.to_string().len();
            (width + 1).max(4) // At least 4 chars wide, +1 for space
        } else {
            0
        };

        let text_start_col = window.x as usize + line_number_width;
        let text_width = window.width.saturating_sub(line_number_width as u16) as usize;

        // Render lines within the window bounds
        for row in 0..content_height {
            let buffer_row = window.viewport_top + row;
            let screen_row = window.y as usize + row;

            // Move cursor to the start of this line within the window
            terminal.queue_move_cursor(Position::new(screen_row, window.x as usize))?;
            
            // Instead of clearing the entire line, clear only the window area
            // by overwriting with spaces
            let spaces = " ".repeat(window.width as usize);
            terminal.queue_print(&spaces)?;
            
            // Move back to the start of the window for actual content rendering
            terminal.queue_move_cursor(Position::new(screen_row, window.x as usize))?;

            // Check if this is the cursor line for highlighting
            let is_cursor_line = self.show_cursor_line && buffer_row == buffer.cursor.row;

            // Set cursor line background if enabled using theme
            if is_cursor_line {
                terminal.queue_set_bg_color(self.theme.cursor_line_bg)?;
            }

            if buffer_row < buffer.lines.len() {
                // Render line number if enabled
                if self.show_line_numbers || self.show_relative_numbers {
                    self.render_line_number(
                        terminal,
                        buffer,
                        buffer_row,
                        line_number_width,
                        is_cursor_line,
                    )?;
                }

                // Move to text area within the window
                terminal.queue_move_cursor(Position::new(screen_row, text_start_col))?;

                let line = &buffer.lines[buffer_row];

                // Check for syntax highlighting
                if let Some(highlights) = editor_state.syntax_highlights.get(&buffer_row) {
                    self.render_highlighted_line(terminal, line, highlights, text_width)?;
                } else {
                    // Render line without syntax highlighting
                    let display_line = if line.len() > text_width {
                        &line[..text_width]
                    } else {
                        line
                    };
                    terminal.queue_print(display_line)?;
                }
            } else {
                // Empty line - render line number if enabled
                if self.show_line_numbers || self.show_relative_numbers {
                    self.render_line_number(
                        terminal,
                        buffer,
                        buffer_row,
                        line_number_width,
                        is_cursor_line,
                    )?;
                }

                // Move to text area and show tilde for empty lines (like Vim)
                terminal.queue_move_cursor(Position::new(screen_row, text_start_col))?;
                if !is_cursor_line {
                    terminal.queue_set_fg_color(self.theme.empty_line)?;
                }
                terminal.queue_print("~")?;
            }

            // Reset colors after each line
            terminal.queue_reset_color()?;
        }

        Ok(())
    }

    fn render_window_border(
        &self,
        terminal: &mut Terminal,
        window: &crate::window::Window,
        is_active: bool,
    ) -> io::Result<()> {
        // Draw border around the window (simple ASCII border)
        let border_char = if is_active { '|' } else { '│' };

        // Right border
        if window.x + window.width < terminal.size().0 {
            for y in window.y..window.y + window.height {
                terminal.queue_move_cursor(Position::new(
                    y as usize,
                    (window.x + window.width) as usize,
                ))?;
                terminal.queue_print(&border_char.to_string())?;
            }
        }

        // Bottom border
        if window.y + window.height < terminal.size().1.saturating_sub(2) {
            terminal.queue_move_cursor(Position::new(
                (window.y + window.height) as usize,
                window.x as usize,
            ))?;
            let border = "─".repeat(window.width as usize);
            terminal.queue_print(&border)?;
        }

        Ok(())
    }

    fn position_cursor(
        &self,
        terminal: &mut Terminal,
        editor_state: &EditorRenderState,
    ) -> io::Result<()> {
        if let Some(current_window) = editor_state.window_manager.current_window() {
            // Get the buffer for the current window
            let buffer = if let Some(buffer_id) = current_window.buffer_id {
                editor_state.all_buffers.get(&buffer_id)
            } else {
                return Ok(());
            };

            if let Some(buffer) = buffer {
                let content_height = current_window.content_height();

                // Calculate line number column width
                let line_number_width = if self.show_line_numbers || self.show_relative_numbers {
                    let max_line_num = buffer.lines.len();
                    let width = max_line_num.to_string().len();
                    (width + 1).max(4) // At least 4 chars wide, +1 for space
                } else {
                    0
                };

                // Calculate screen cursor position relative to the current window
                let screen_row = buffer
                    .cursor
                    .row
                    .saturating_sub(current_window.viewport_top);
                let screen_col = buffer.cursor.col + line_number_width;

                // Ensure cursor is within window bounds
                if screen_row < content_height {
                    let final_row = current_window.y as usize + screen_row;
                    let final_col = current_window.x as usize + screen_col;
                    terminal.queue_move_cursor(Position::new(final_row, final_col))?;
                }
            }
        }
        Ok(())
    }

    fn render_highlighted_line(
        &self,
        terminal: &mut Terminal,
        line: &str,
        highlights: &[HighlightRange],
        max_width: usize,
    ) -> io::Result<()> {
        let line_bytes = line.as_bytes();
        let mut current_pos = 0;

        // Truncate highlights to fit within max_width
        let display_len = std::cmp::min(line.len(), max_width);

        for highlight in highlights {
            let start = std::cmp::min(highlight.start, display_len);
            let end = std::cmp::min(highlight.end, display_len);

            if start >= display_len {
                break;
            }

            // Print any text before this highlight
            if current_pos < start {
                let text_before =
                    std::str::from_utf8(&line_bytes[current_pos..start]).unwrap_or("");
                terminal.queue_print(text_before)?;
            }

            // Apply highlight style and print highlighted text
            if let Some(color) = highlight.style.to_color() {
                terminal.queue_set_fg_color(color)?;
            }

            if highlight.style.bold {
                // Note: Bold support would need to be added to terminal module
            }

            let highlighted_text = std::str::from_utf8(&line_bytes[start..end]).unwrap_or("");
            terminal.queue_print(highlighted_text)?;

            // Reset color
            terminal.queue_reset_color()?;

            current_pos = end;
        }

        // Print any remaining text after the last highlight
        if current_pos < display_len {
            let remaining_text =
                std::str::from_utf8(&line_bytes[current_pos..display_len]).unwrap_or("");
            terminal.queue_print(remaining_text)?;
        }

        Ok(())
    }

    fn render_line_number(
        &self,
        terminal: &mut Terminal,
        buffer: &crate::buffer::Buffer,
        buffer_row: usize,
        width: usize,
        is_cursor_line: bool,
    ) -> io::Result<()> {
        // Set line number colors using theme - highlight current line number if on cursor line
        if is_cursor_line && self.show_cursor_line {
            terminal.queue_set_fg_color(self.theme.line_number_current)?;
            terminal.queue_set_bg_color(self.theme.cursor_line_bg)?;
        } else {
            terminal.queue_set_fg_color(self.theme.line_number)?;
        }

        if buffer_row < buffer.lines.len() {
            let line_num = if self.show_relative_numbers {
                let current_line = buffer.cursor.row;
                if buffer_row == current_line {
                    // Show absolute line number for current line
                    buffer_row + 1
                } else {
                    // Show relative distance
                    if buffer_row > current_line {
                        buffer_row - current_line
                    } else {
                        current_line - buffer_row
                    }
                }
            } else {
                // Show absolute line numbers
                buffer_row + 1
            };

            let line_num_str = format!("{:>width$} ", line_num, width = width - 1);
            terminal.queue_print(&line_num_str)?;
        } else {
            // Empty line - just print spaces
            let spaces = " ".repeat(width);
            terminal.queue_print(&spaces)?;
        }

        // Don't reset color here - let the caller handle it
        Ok(())
    }

    fn render_status_line(
        &self,
        terminal: &mut Terminal,
        editor_state: &EditorRenderState,
        width: u16,
        height: u16,
    ) -> io::Result<()> {
        let status_row = height.saturating_sub(2);
        terminal.queue_move_cursor(Position::new(status_row as usize, 0))?;

        // Clear the status line first
        terminal.queue_clear_line()?;

        // Set status line colors using theme
        let status_color = if editor_state
            .current_buffer
            .as_ref()
            .map_or(false, |b| b.modified)
        {
            self.theme.status_modified
        } else {
            self.theme.status_bg
        };
        terminal.queue_set_bg_color(status_color)?;
        terminal.queue_set_fg_color(self.theme.status_fg)?;

        let mut status_text = String::new();

        // Mode indicator
        status_text.push_str(&format!(" {} ", editor_state.mode));

        // Buffer information
        if editor_state.buffer_count > 1 {
            if let Some(buffer_id) = editor_state.current_buffer_id {
                status_text.push_str(&format!(" [{}] ", buffer_id));
            }
        }

        // File information
        if let Some(buffer) = &editor_state.current_buffer {
            if let Some(path) = &buffer.file_path {
                status_text.push_str(&format!(" {} ", path.display()));
            } else {
                status_text.push_str(" [No Name] ");
            }

            if buffer.modified {
                status_text.push_str("[+] ");
            }

            // Cursor position
            status_text.push_str(&format!(
                "{}:{} ",
                buffer.cursor.row + 1,
                buffer.cursor.col + 1
            ));
        }

        // Status message
        if !editor_state.status_message.is_empty() {
            status_text.push_str(&format!(" | {}", editor_state.status_message));
        }

        // Pad the status line to full width
        let padding = width as usize - status_text.len().min(width as usize);
        status_text.push_str(&" ".repeat(padding));

        // Truncate if too long
        if status_text.len() > width as usize {
            status_text.truncate(width as usize);
        }

        terminal.queue_print(&status_text)?;
        terminal.queue_reset_color()?;

        Ok(())
    }

    fn render_command_line(
        &self,
        terminal: &mut Terminal,
        editor_state: &EditorRenderState,
        width: u16,
        height: u16,
    ) -> io::Result<()> {
        let command_row = height.saturating_sub(1);
        terminal.queue_move_cursor(Position::new(command_row as usize, 0))?;

        // Clear the command line first and set theme colors
        terminal.queue_clear_line()?;
        terminal.queue_set_bg_color(self.theme.command_line_bg)?;
        terminal.queue_set_fg_color(self.theme.command_line_fg)?;

        let command_text = &editor_state.command_line;

        // Truncate if too long
        let display_text = if command_text.len() > width as usize {
            &command_text[..width as usize]
        } else {
            command_text
        };

        terminal.queue_print(display_text)?;
        terminal.queue_reset_color()?;

        Ok(())
    }

    /// Get the current viewport top position
    pub fn viewport_top(&self) -> usize {
        self.viewport_top
    }

    /// Get the current viewport range
    pub fn viewport_range(&self, height: u16) -> (usize, usize) {
        let content_height = height.saturating_sub(2) as usize;
        (self.viewport_top, content_height)
    }

    // Scrolling methods
    pub fn scroll_down_line(&mut self) {
        self.viewport_top = self.viewport_top.saturating_add(1);
    }

    pub fn scroll_up_line(&mut self) {
        self.viewport_top = self.viewport_top.saturating_sub(1);
    }

    pub fn scroll_down_page(&mut self) {
        // Use a more conservative page scroll (like vim's Ctrl+f)
        let page_size = 20; // Default to 20 lines if we can't get terminal size
        self.viewport_top = self.viewport_top.saturating_add(page_size);
    }

    pub fn scroll_up_page(&mut self) {
        // Use a more conservative page scroll (like vim's Ctrl+b)
        let page_size = 20; // Default to 20 lines if we can't get terminal size
        self.viewport_top = self.viewport_top.saturating_sub(page_size);
    }

    pub fn scroll_down_half_page(&mut self) {
        // Vim-style half-page scroll (Ctrl+d)
        let half_page_size = 10; // Default to 10 lines
        self.viewport_top = self.viewport_top.saturating_add(half_page_size);
    }

    pub fn scroll_up_half_page(&mut self) {
        // Vim-style half-page scroll (Ctrl+u)
        let half_page_size = 10; // Default to 10 lines
        self.viewport_top = self.viewport_top.saturating_sub(half_page_size);
    }

    // Centering methods (z commands in Vim)
    pub fn center_cursor(&mut self, cursor_row: usize, terminal_height: u16) {
        // zz: Center current line in viewport
        let content_height = terminal_height.saturating_sub(2) as usize; // Reserve space for status and command line
        let half_height = content_height / 2;

        // Set viewport so cursor line is in the middle
        self.viewport_top = cursor_row.saturating_sub(half_height);
    }

    pub fn cursor_to_top(&mut self, cursor_row: usize) {
        // zt: Move current line to top of viewport
        self.viewport_top = cursor_row;
    }

    pub fn cursor_to_bottom(&mut self, cursor_row: usize, terminal_height: u16) {
        // zb: Move current line to bottom of viewport
        let content_height = terminal_height.saturating_sub(2) as usize; // Reserve space for status and command line

        // Set viewport so cursor line is at the bottom
        self.viewport_top = cursor_row.saturating_sub(content_height.saturating_sub(1));
    }

    pub fn set_viewport_top(&mut self, viewport_top: usize) {
        self.viewport_top = viewport_top;
    }
}
