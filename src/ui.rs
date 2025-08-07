use crate::editor::EditorRenderState;
use crate::mode::{Mode, Position};
use crate::syntax::HighlightRange;
use crate::terminal::Terminal;
use crate::theme::{SyntaxTheme, ThemeConfig, UITheme};
use log::{debug, warn};
use std::io;

#[cfg(test)]
mod tests;

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
    /// Current syntax theme from themes.toml
    syntax_theme: SyntaxTheme,
}

impl UI {
    pub fn new() -> Self {
        // Load theme configuration from themes.toml
        let theme_config = ThemeConfig::load();
        let current_theme = theme_config.get_current_theme();

        Self {
            viewport_top: 0,
            show_line_numbers: true,            // Enable by default like Vim
            show_relative_numbers: false,       // Disabled by default
            show_cursor_line: true,             // Enable by default
            theme: current_theme.ui,            // Use theme from themes.toml
            syntax_theme: current_theme.syntax, // Use syntax theme from themes.toml
        }
    }

    /// Set the UI theme by loading from themes.toml
    pub fn set_theme(&mut self, theme_name: &str) {
        debug!("Setting UI theme to: '{}'", theme_name);
        let theme_config = ThemeConfig::load();
        if let Some(complete_theme) = theme_config.get_theme(theme_name) {
            debug!("Successfully loaded theme: '{}'", theme_name);
            self.theme = complete_theme.ui;
            self.syntax_theme = complete_theme.syntax;
        } else {
            warn!("Theme '{}' not found, using default theme", theme_name);
            // Fallback to default theme if theme not found
            let default_theme = theme_config.get_current_theme();
            self.theme = default_theme.ui;
            self.syntax_theme = default_theme.syntax;
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

        // Set the background color for the entire screen
        terminal.queue_set_bg_color(self.theme.background)?;
        terminal.queue_clear_screen()?;

        // Render all windows
        self.render_windows(terminal, editor_state)?;

        // Render status line
        self.render_status_line(terminal, editor_state, width, height)?;

        // Render command line if in command or search mode
        if editor_state.mode == Mode::Command || editor_state.mode == Mode::Search {
            self.render_command_line(terminal, editor_state, width, height)?;
        }

        // Render command completion popup if active
        if editor_state.mode == Mode::Command && editor_state.command_completion.should_show() {
            self.render_completion_popup(terminal, editor_state, width, height)?;
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

            // Check if this is the cursor line for highlighting (only in the active window)
            let is_active_window = editor_state.current_window_id == Some(window.id);
            let is_cursor_line =
                self.show_cursor_line && is_active_window && buffer_row == buffer.cursor.row;

            // Set background color for this line (cursor line background or normal background)
            if is_cursor_line {
                terminal.queue_set_bg_color(self.theme.cursor_line_bg)?;
            } else {
                terminal.queue_set_bg_color(self.theme.background)?;
            }

            // Clear the window area with the background color set
            let spaces = " ".repeat(window.width as usize);
            terminal.queue_print(&spaces)?;

            // Move back to the start of the window for actual content rendering
            terminal.queue_move_cursor(Position::new(screen_row, window.x as usize))?;

            if buffer_row < buffer.lines.len() {
                // Render line number if enabled
                if self.show_line_numbers || self.show_relative_numbers {
                    self.render_line_number(
                        terminal,
                        buffer,
                        buffer_row,
                        line_number_width,
                        is_cursor_line,
                        is_active_window,
                    )?;
                }

                // Move to text area within the window
                terminal.queue_move_cursor(Position::new(screen_row, text_start_col))?;

                let line = &buffer.lines[buffer_row];

                // Track how much content we've rendered for cursor line filling
                let content_rendered = if let Some(highlights) =
                    editor_state.syntax_highlights.get(&(buffer.id, buffer_row))
                {
                    // Debug: Show we have highlights
                    if log::log_enabled!(log::Level::Debug) && highlights.len() > 0 {
                        debug!(
                            "UI: Rendering line {} with {} highlights: '{}'",
                            buffer_row,
                            highlights.len(),
                            line.chars().take(30).collect::<String>()
                        );

                        // Show what's actually being highlighted
                        for (i, highlight) in highlights.iter().enumerate() {
                            let highlighted_text = &line
                                [highlight.start.min(line.len())..highlight.end.min(line.len())];
                            debug!(
                                "  Highlight {}: '{}' ({}-{}) color: {:?}",
                                i,
                                highlighted_text,
                                highlight.start,
                                highlight.end,
                                highlight.style.fg_color
                            );
                        }
                    }
                    self.render_highlighted_line(
                        terminal,
                        line,
                        highlights,
                        text_width,
                        is_cursor_line,
                    )?
                } else {
                    // Debug: Show we're missing highlights
                    if log::log_enabled!(log::Level::Debug) {
                        debug!(
                            "UI: No highlights for line {} (buffer {}): '{}'",
                            buffer_row,
                            buffer.id,
                            line.chars().take(30).collect::<String>()
                        );
                    }
                    // Render line without syntax highlighting - use theme's default text color
                    terminal.queue_set_fg_color(self.syntax_theme.get_default_text_color())?;
                    let display_line = if line.len() > text_width {
                        &line[..text_width]
                    } else {
                        line
                    };
                    terminal.queue_print(display_line)?;
                    display_line.len()
                };

                // Fill remaining width with appropriate background
                if content_rendered < text_width {
                    let remaining_width = text_width - content_rendered;
                    let filler = " ".repeat(remaining_width);
                    terminal.queue_print(&filler)?;
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
                        is_active_window,
                    )?;
                }

                // Move to text area and show tilde for empty lines (like Vim)
                terminal.queue_move_cursor(Position::new(screen_row, text_start_col))?;
                if !is_cursor_line {
                    terminal.queue_set_fg_color(self.theme.empty_line)?;
                }
                terminal.queue_print("~")?;

                // Fill remaining width with appropriate background
                let remaining_width = text_width - 1; // -1 for the tilde character
                if remaining_width > 0 {
                    let filler = " ".repeat(remaining_width);
                    terminal.queue_print(&filler)?;
                }
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
        is_cursor_line: bool,
    ) -> io::Result<usize> {
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
                // Set default text color for unhighlighted text
                terminal.queue_set_fg_color(self.syntax_theme.get_default_text_color())?;
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

            // Don't reset colors - just ensure background is set correctly for cursor line
            // The foreground color will be managed for each text segment
            if is_cursor_line && self.show_cursor_line {
                terminal.queue_set_bg_color(self.theme.cursor_line_bg)?;
            } else {
                terminal.queue_set_bg_color(self.theme.background)?;
            }

            current_pos = end;
        }

        // Print any remaining text after the last highlight
        if current_pos < display_len {
            // Set default text color for unhighlighted text
            terminal.queue_set_fg_color(self.syntax_theme.get_default_text_color())?;
            let remaining_text =
                std::str::from_utf8(&line_bytes[current_pos..display_len]).unwrap_or("");
            terminal.queue_print(remaining_text)?;
        }

        Ok(display_len)
    }

    fn render_line_number(
        &self,
        terminal: &mut Terminal,
        buffer: &crate::buffer::Buffer,
        buffer_row: usize,
        width: usize,
        is_cursor_line: bool,
        is_active_window: bool,
    ) -> io::Result<()> {
        // Set line number colors using theme - highlight current line number if on cursor line
        if is_cursor_line && self.show_cursor_line {
            terminal.queue_set_fg_color(self.theme.line_number_current)?;
            terminal.queue_set_bg_color(self.theme.cursor_line_bg)?;
        } else {
            terminal.queue_set_fg_color(self.theme.line_number)?;
            terminal.queue_set_bg_color(self.theme.background)?;
        }

        if buffer_row < buffer.lines.len() {
            let line_num = if self.show_relative_numbers && is_active_window {
                // Only show relative numbers in the active window
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
                // Show absolute line numbers (for inactive windows or when relative numbers are disabled)
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

    fn render_completion_popup(
        &self,
        terminal: &mut Terminal,
        editor_state: &EditorRenderState,
        width: u16,
        height: u16,
    ) -> io::Result<()> {
        if !editor_state.command_completion.should_show() {
            return Ok(());
        }

        // Get completion menu dimensions from config
        let menu_width = editor_state
            .config
            .interface
            .completion_menu_width
            .min(width - 2);
        let max_menu_height = editor_state.config.interface.completion_menu_height as usize;
        let menu_height = editor_state
            .command_completion
            .matches
            .len()
            .min(max_menu_height)
            .min((height - 3) as usize); // Reserve space for status and command line

        if menu_height == 0 {
            return Ok(());
        }

        // Position the popup above the command line
        let popup_row = height.saturating_sub(2); // One line above command line
        let popup_col = 0; // Start at left edge

        // Get visible completion items
        let visible_items = editor_state.command_completion.visible_items(menu_height);
        let selected_index = editor_state
            .command_completion
            .visible_selected_index(menu_height);

        // Render the popup background and border
        for i in 0..menu_height {
            let row = popup_row.saturating_sub(menu_height as u16) + i as u16;
            terminal.queue_move_cursor(Position::new(row as usize, popup_col as usize))?;

            if i < visible_items.len() {
                let item = &visible_items[i];
                let is_selected = i == selected_index;

                // Set colors based on selection
                if is_selected {
                    terminal.queue_set_bg_color(self.theme.selection_bg)?;
                    terminal.queue_set_fg_color(self.theme.command_line_fg)?;
                } else {
                    terminal.queue_set_bg_color(self.theme.command_line_bg)?;
                    terminal.queue_set_fg_color(self.theme.command_line_fg)?;
                }

                // Format the completion item
                let display_text = if item.text.len() + 2 <= menu_width as usize {
                    format!(" {}", item.text)
                } else {
                    format!(" {}", &item.text[..menu_width.saturating_sub(2) as usize])
                };

                // Pad to exact width and print
                let padded_text = format!("{:width$}", display_text, width = menu_width as usize);
                terminal.queue_print(&padded_text)?;

                // Reset colors immediately after printing each line
                terminal.queue_reset_color()?;
            } else {
                // Empty row - set background color and fill with spaces
                terminal.queue_set_bg_color(self.theme.command_line_bg)?;
                terminal.queue_set_fg_color(self.theme.command_line_fg)?;
                let empty_line = " ".repeat(menu_width as usize);
                terminal.queue_print(&empty_line)?;

                // Reset colors immediately after printing each line
                terminal.queue_reset_color()?;
            }
        }

        // Final color reset to ensure no artifacts
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

    pub fn set_viewport_top(&mut self, viewport_top: usize) {
        self.viewport_top = viewport_top;
    }
}
