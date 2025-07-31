use crate::mode::{Position, Selection};
use anyhow::Result;
use std::collections::VecDeque;
use std::path::PathBuf;

/// Represents a text buffer with content and metadata
#[derive(Debug, Clone)]
pub struct Buffer {
    /// Buffer ID
    pub id: usize,
    /// File path (None for unnamed buffers)
    pub file_path: Option<PathBuf>,
    /// Buffer content as lines
    pub lines: Vec<String>,
    /// Whether the buffer has been modified
    pub modified: bool,
    /// Cursor position
    pub cursor: Position,
    /// Visual selection (if any)
    pub selection: Option<Selection>,
    /// Undo stack
    pub undo_stack: VecDeque<BufferState>,
    /// Redo stack
    pub redo_stack: VecDeque<BufferState>,
    /// Buffer type (normal, help, quickfix, etc.)
    pub buffer_type: BufferType,
}

#[derive(Debug, Clone)]
pub struct BufferState {
    pub lines: Vec<String>,
    pub cursor: Position,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BufferType {
    Normal,
    Help,
    Quickfix,
    Terminal,
    Scratch,
}

impl Buffer {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            file_path: None,
            lines: vec![String::new()],
            modified: false,
            cursor: Position::zero(),
            selection: None,
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            buffer_type: BufferType::Normal,
        }
    }

    pub fn from_file(id: usize, path: PathBuf) -> Result<Self> {
        let content = std::fs::read_to_string(&path)?;
        let lines: Vec<String> = if content.is_empty() {
            vec![String::new()]
        } else {
            content.lines().map(String::from).collect()
        };

        Ok(Self {
            id,
            file_path: Some(path),
            lines,
            modified: false,
            cursor: Position::zero(),
            selection: None,
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            buffer_type: BufferType::Normal,
        })
    }

    pub fn insert_char(&mut self, ch: char) {
        self.save_state();

        if self.cursor.row >= self.lines.len() {
            self.lines.push(String::new());
        }

        let line = &mut self.lines[self.cursor.row];
        if self.cursor.col <= line.len() {
            line.insert(self.cursor.col, ch);
            self.cursor.col += 1;
            self.modified = true;
        }
    }

    pub fn insert_line_break(&mut self) {
        self.save_state();

        if self.cursor.row >= self.lines.len() {
            self.lines.push(String::new());
            self.cursor.row = self.lines.len() - 1;
            self.cursor.col = 0;
        } else {
            let line = &mut self.lines[self.cursor.row];
            let new_line = line.split_off(self.cursor.col);
            self.lines.insert(self.cursor.row + 1, new_line);
            self.cursor.row += 1;
            self.cursor.col = 0;
        }
        self.modified = true;
    }

    pub fn delete_char(&mut self) -> bool {
        if self.cursor.col > 0 {
            self.save_state();
            let line = &mut self.lines[self.cursor.row];
            if self.cursor.col <= line.len() {
                line.remove(self.cursor.col - 1);
                self.cursor.col -= 1;
                self.modified = true;
                return true;
            }
        } else if self.cursor.row > 0 {
            // Join with previous line
            self.save_state();
            let current_line = self.lines.remove(self.cursor.row);
            self.cursor.row -= 1;
            self.cursor.col = self.lines[self.cursor.row].len();
            self.lines[self.cursor.row].push_str(&current_line);
            self.modified = true;
            return true;
        }
        false
    }

    pub fn move_cursor(&mut self, new_pos: Position) {
        let max_row = if self.lines.is_empty() {
            0
        } else {
            self.lines.len() - 1
        };
        let row = new_pos.row.min(max_row);
        let max_col = if row < self.lines.len() {
            self.lines[row].len()
        } else {
            0
        };
        let col = new_pos.col.min(max_col);

        self.cursor = Position::new(row, col);
    }

    pub fn save_state(&mut self) {
        let state = BufferState {
            lines: self.lines.clone(),
            cursor: self.cursor,
        };

        self.undo_stack.push_back(state);
        self.redo_stack.clear();

        // Limit undo stack size
        if self.undo_stack.len() > 1000 {
            self.undo_stack.pop_front();
        }
    }

    pub fn undo(&mut self) -> bool {
        if let Some(state) = self.undo_stack.pop_back() {
            let current_state = BufferState {
                lines: self.lines.clone(),
                cursor: self.cursor,
            };
            self.redo_stack.push_back(current_state);

            self.lines = state.lines;
            self.cursor = state.cursor;
            self.modified = true;
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        if let Some(state) = self.redo_stack.pop_back() {
            let current_state = BufferState {
                lines: self.lines.clone(),
                cursor: self.cursor,
            };
            self.undo_stack.push_back(current_state);

            self.lines = state.lines;
            self.cursor = state.cursor;
            self.modified = true;
            true
        } else {
            false
        }
    }

    pub fn get_line(&self, row: usize) -> Option<&String> {
        self.lines.get(row)
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn save(&mut self) -> Result<()> {
        if let Some(path) = &self.file_path {
            let content = self.lines.join("\n");
            std::fs::write(path, content)?;
            self.modified = false;
        }
        Ok(())
    }

    /// Delete character at cursor position (like 'x' in Vim)
    pub fn delete_char_at_cursor(&mut self) -> bool {
        if self.cursor.row < self.lines.len() {
            if self.cursor.col < self.lines[self.cursor.row].len() {
                self.save_state();
                let line = &mut self.lines[self.cursor.row];
                line.remove(self.cursor.col);
                self.modified = true;
                return true;
            }
        }
        false
    }

    /// Delete character before cursor (like 'X' in Vim)
    pub fn delete_char_before_cursor(&mut self) -> bool {
        if self.cursor.col > 0 {
            self.save_state();
            let line = &mut self.lines[self.cursor.row];
            if self.cursor.col <= line.len() {
                line.remove(self.cursor.col - 1);
                self.cursor.col -= 1;
                self.modified = true;
                return true;
            }
        }
        false
    }

    /// Delete entire line (like 'dd' in Vim)
    pub fn delete_line(&mut self) -> bool {
        if !self.lines.is_empty() && self.cursor.row < self.lines.len() {
            self.save_state();

            // If this is the only line, just clear it
            if self.lines.len() == 1 {
                self.lines[0].clear();
                self.cursor.col = 0;
            } else {
                // Remove the line
                self.lines.remove(self.cursor.row);

                // Adjust cursor position
                if self.cursor.row >= self.lines.len() {
                    self.cursor.row = self.lines.len() - 1;
                }
                self.cursor.col = 0;
            }

            self.modified = true;
            return true;
        }
        false
    }

    /// Move cursor to start of next word
    pub fn move_to_next_word(&mut self) {
        if self.cursor.row >= self.lines.len() {
            return;
        }

        let line = &self.lines[self.cursor.row];
        let mut pos = self.cursor.col;

        // Skip current word
        while pos < line.len() && !line.chars().nth(pos).unwrap_or(' ').is_whitespace() {
            pos += 1;
        }

        // Skip whitespace
        while pos < line.len() && line.chars().nth(pos).unwrap_or(' ').is_whitespace() {
            pos += 1;
        }

        // If we reached end of line, go to next line
        if pos >= line.len() && self.cursor.row + 1 < self.lines.len() {
            self.cursor.row += 1;
            self.cursor.col = 0;
        } else {
            self.cursor.col = pos.min(line.len());
        }
    }

    /// Move cursor to start of previous word
    pub fn move_to_previous_word(&mut self) {
        if self.cursor.col > 0 {
            let line = &self.lines[self.cursor.row];
            let mut pos = self.cursor.col - 1;

            // Skip whitespace
            while pos > 0 && line.chars().nth(pos).unwrap_or(' ').is_whitespace() {
                pos -= 1;
            }

            // Skip word
            while pos > 0 && !line.chars().nth(pos - 1).unwrap_or(' ').is_whitespace() {
                pos -= 1;
            }

            self.cursor.col = pos;
        } else if self.cursor.row > 0 {
            // Go to end of previous line
            self.cursor.row -= 1;
            if let Some(line) = self.lines.get(self.cursor.row) {
                self.cursor.col = line.len();
            }
        }
    }

    /// Move cursor to end of current word
    pub fn move_to_word_end(&mut self) {
        if self.cursor.row >= self.lines.len() {
            return;
        }

        let line = &self.lines[self.cursor.row];
        if self.cursor.col >= line.len() {
            return;
        }

        let mut pos = self.cursor.col;

        // If we're on whitespace, skip to next word first
        if line.chars().nth(pos).unwrap_or(' ').is_whitespace() {
            while pos < line.len() && line.chars().nth(pos).unwrap_or(' ').is_whitespace() {
                pos += 1;
            }
        }

        // Move to end of current word
        while pos < line.len() && !line.chars().nth(pos).unwrap_or(' ').is_whitespace() {
            pos += 1;
        }

        if pos > 0 {
            pos -= 1; // Back up to last character of word
        }

        self.cursor.col = pos.min(line.len().saturating_sub(1));
    }
}
