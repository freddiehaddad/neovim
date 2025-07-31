use crate::editor::EditorRenderState;
use crate::mode::{Mode, Position};
use crate::terminal::Terminal;
use crossterm::style::Color;
use std::io;

pub struct UI {
    /// Top row of the current viewport
    viewport_top: usize,
    /// Show absolute line numbers
    pub show_line_numbers: bool,
    /// Show relative line numbers
    pub show_relative_numbers: bool,
}

impl UI {
    pub fn new() -> Self {
        Self {
            viewport_top: 0,
            show_line_numbers: true,      // Enable by default like Vim
            show_relative_numbers: false, // Disabled by default
        }
    }

    fn update_viewport(&mut self, buffer: &crate::buffer::Buffer, height: u16) -> (usize, usize) {
        let content_height = height.saturating_sub(2) as usize; // Reserve space for status and command line

        // Check if cursor is outside current viewport
        let viewport_bottom = self.viewport_top + content_height;

        if buffer.cursor.row < self.viewport_top {
            // Cursor moved above viewport - scroll up
            self.viewport_top = buffer.cursor.row;
        } else if buffer.cursor.row >= viewport_bottom {
            // Cursor moved below viewport - scroll down
            self.viewport_top = buffer.cursor.row.saturating_sub(content_height - 1);
        }
        // If cursor is within viewport, don't change viewport_top

        (self.viewport_top, content_height)
    }

    pub fn render(
        &mut self,
        terminal: &mut Terminal,
        editor_state: &EditorRenderState,
    ) -> io::Result<()> {
        let (width, height) = terminal.size();

        // Use queued operations to reduce flicker
        terminal.hide_cursor()?;

        // Update viewport based on cursor position first
        if let Some(buffer) = &editor_state.current_buffer {
            self.update_viewport(buffer, height);
        }

        // Render buffer content
        if let Some(buffer) = &editor_state.current_buffer {
            self.render_buffer(terminal, buffer, width, height)?;
        }

        // Render status line
        self.render_status_line(terminal, editor_state, width, height)?;

        // Render command line if in command or search mode
        if editor_state.mode == Mode::Command || editor_state.mode == Mode::Search {
            self.render_command_line(terminal, editor_state, width, height)?;
        }

        // Position cursor and show it
        if let Some(buffer) = &editor_state.current_buffer {
            let content_height = height.saturating_sub(2) as usize;

            // Calculate line number column width
            let line_number_width = if self.show_line_numbers || self.show_relative_numbers {
                let max_line_num = buffer.lines.len();
                let width = max_line_num.to_string().len();
                (width + 1).max(4) // At least 4 chars wide, +1 for space
            } else {
                0
            };

            // Calculate screen cursor position relative to the current viewport
            let screen_row = buffer.cursor.row.saturating_sub(self.viewport_top);
            let screen_col = buffer.cursor.col + line_number_width;

            // Ensure cursor is within visible bounds
            if screen_row < content_height {
                terminal.move_cursor(Position::new(screen_row, screen_col))?;
            }
        }

        terminal.show_cursor()?;
        Ok(())
    }

    fn render_buffer(
        &self,
        terminal: &mut Terminal,
        buffer: &crate::buffer::Buffer,
        width: u16,
        height: u16,
    ) -> io::Result<()> {
        let content_height = height.saturating_sub(2) as usize;
        let start_row = self.viewport_top;

        // Calculate line number column width
        let line_number_width = if self.show_line_numbers || self.show_relative_numbers {
            let max_line_num = buffer.lines.len();
            let width = max_line_num.to_string().len();
            (width + 1).max(4) // At least 4 chars wide, +1 for space
        } else {
            0
        };

        let text_start_col = line_number_width;
        let text_width = width.saturating_sub(text_start_col as u16) as usize;

        for (screen_row, buffer_row) in (start_row..).take(content_height).enumerate() {
            terminal.move_cursor(Position::new(screen_row, 0))?;
            terminal.clear_line()?; // Clear only this line instead of whole screen

            // Render line numbers
            if self.show_line_numbers || self.show_relative_numbers {
                self.render_line_number(terminal, buffer, buffer_row, line_number_width)?;
            }

            // Move to text area
            terminal.move_cursor(Position::new(screen_row, text_start_col))?;

            if let Some(line) = buffer.get_line(buffer_row) {
                // Truncate line if it's too long
                let display_line = if line.len() > text_width {
                    &line[..text_width]
                } else {
                    line
                };
                terminal.print(display_line)?;
            } else {
                // Show tilde for empty lines (like Vim)
                terminal.set_foreground_color(Color::Blue)?;
                terminal.print("~")?;
                terminal.reset_color()?;
            }
        }

        Ok(())
    }

    fn render_line_number(
        &self,
        terminal: &mut Terminal,
        buffer: &crate::buffer::Buffer,
        buffer_row: usize,
        width: usize,
    ) -> io::Result<()> {
        // Set line number colors (gray by default)
        terminal.set_foreground_color(Color::DarkGrey)?;

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
            terminal.print(&line_num_str)?;
        } else {
            // Empty line - just print spaces
            let spaces = " ".repeat(width);
            terminal.print(&spaces)?;
        }

        terminal.reset_color()?;
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
        terminal.move_cursor(Position::new(status_row as usize, 0))?;

        // Clear the status line first
        terminal.clear_line()?;

        // Set status line colors
        terminal.set_background_color(Color::White)?;
        terminal.set_foreground_color(Color::Black)?;

        let mut status_text = String::new();

        // Mode indicator
        status_text.push_str(&format!(" {} ", editor_state.mode));

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

        terminal.print(&status_text)?;
        terminal.reset_color()?;

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
        terminal.move_cursor(Position::new(command_row as usize, 0))?;

        // Clear the command line first
        terminal.clear_line()?;

        let command_text = &editor_state.command_line;

        // Truncate if too long
        let display_text = if command_text.len() > width as usize {
            &command_text[..width as usize]
        } else {
            command_text
        };

        terminal.print(display_text)?;

        Ok(())
    }
}
