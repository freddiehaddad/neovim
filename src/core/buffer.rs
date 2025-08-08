use crate::core::mode::{Position, Selection};
use anyhow::Result;
use log::{debug, info, trace, warn};
use std::collections::VecDeque;
use std::path::PathBuf;

/// Types of content that can be yanked
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YankType {
    Character, // Character-wise yank (like yanking a word)
    Line,      // Line-wise yank (like yy)
    Block,     // Block-wise yank (visual block mode)
}

/// Content stored in the clipboard
#[derive(Debug, Clone)]
pub struct ClipboardContent {
    pub text: String,
    pub yank_type: YankType,
}

impl Default for ClipboardContent {
    fn default() -> Self {
        Self {
            text: String::new(),
            yank_type: YankType::Character,
        }
    }
}

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
    pub undo_stack: VecDeque<BufferDelta>,
    /// Redo stack
    pub redo_stack: VecDeque<BufferDelta>,
    /// Buffer type (normal, help, quickfix, etc.)
    pub buffer_type: BufferType,
    /// Clipboard for yank/put operations
    pub clipboard: ClipboardContent,
    /// Maximum number of undo levels to keep
    pub undo_levels: usize,
}

/// Represents a single edit operation for delta-based undo system
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOperation {
    /// Insert text at a position
    Insert { pos: Position, text: String },
    /// Delete text at a position (stores deleted text for undo)
    Delete { pos: Position, text: String },
    /// Replace text at a position
    Replace {
        pos: Position,
        old: String,
        new: String,
    },
}

/// Delta representing changes made to buffer state
#[derive(Debug, Clone)]
pub struct BufferDelta {
    /// The edit operations performed
    pub operations: Vec<EditOperation>,
    /// Cursor position before the edit
    pub cursor_before: Position,
    /// Cursor position after the edit
    pub cursor_after: Position,
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
    pub fn new(id: usize, undo_levels: usize) -> Self {
        debug!(
            "Creating new empty buffer with ID: {} (undo levels: {})",
            id, undo_levels
        );
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
            clipboard: ClipboardContent::default(),
            undo_levels,
        }
    }

    pub fn from_file(id: usize, path: PathBuf, undo_levels: usize) -> Result<Self> {
        info!(
            "Creating buffer {} from file: {:?} (undo levels: {})",
            id, path, undo_levels
        );
        let content = std::fs::read_to_string(&path)?;
        let lines: Vec<String> = if content.is_empty() {
            debug!("File {:?} is empty, creating single empty line", path);
            vec![String::new()]
        } else {
            let line_count = content.lines().count();
            debug!("Loaded {} lines from file: {:?}", line_count, path);
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
            clipboard: ClipboardContent::default(),
            undo_levels,
        })
    }

    pub fn insert_char(&mut self, ch: char) {
        trace!(
            "Inserting character '{}' at position {}:{}",
            ch, self.cursor.row, self.cursor.col
        );

        // Create operation for undo system
        let operation = EditOperation::Insert {
            pos: self.cursor,
            text: ch.to_string(),
        };
        self.save_operation(operation);

        // Perform the actual insertion
        self.insert_char_raw(ch);
        self.modified = true;
    }

    pub fn insert_line_break(&mut self) {
        debug!(
            "Inserting line break at position {}:{}",
            self.cursor.row, self.cursor.col
        );

        // Create operation for undo system
        let operation = EditOperation::Insert {
            pos: self.cursor,
            text: "\n".to_string(),
        };
        self.save_operation(operation);

        // Perform the actual insertion
        self.insert_line_break_raw();
        self.modified = true;
    }

    pub fn delete_char(&mut self) -> bool {
        if self.cursor.col > 0 {
            // Get character to delete for undo
            let line = &self.lines[self.cursor.row];
            if self.cursor.col <= line.len() {
                let deleted_char = line.chars().nth(self.cursor.col - 1).unwrap_or(' ');
                let operation = EditOperation::Delete {
                    pos: Position {
                        row: self.cursor.row,
                        col: self.cursor.col - 1,
                    },
                    text: deleted_char.to_string(),
                };
                self.save_operation(operation);

                self.delete_char_raw();
                self.modified = true;
                return true;
            }
        } else if self.cursor.row > 0 {
            // Join with previous line - delete newline character
            let operation = EditOperation::Delete {
                pos: Position {
                    row: self.cursor.row - 1,
                    col: self.lines[self.cursor.row - 1].len(),
                },
                text: "\n".to_string(),
            };
            self.save_operation(operation);

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

    fn save_operation(&mut self, operation: EditOperation) {
        debug!("Saving edit operation for undo: {:?}", operation);

        // Create the delta with the operation
        let delta = BufferDelta {
            operations: vec![operation],
            cursor_before: self.cursor,
            cursor_after: self.cursor, // Will be updated after the operation
        };

        self.undo_stack.push_back(delta);

        // Clear redo stack when new operation is saved
        self.redo_stack.clear();

        // Limit undo stack size using configured undo_levels
        if self.undo_stack.len() > self.undo_levels {
            self.undo_stack.pop_front();
        }
    }

    fn apply_edit_operation(&mut self, operation: &EditOperation) {
        match operation {
            EditOperation::Insert { pos, text } => {
                self.cursor = *pos;
                for ch in text.chars() {
                    if ch == '\n' {
                        self.insert_line_break_raw();
                    } else {
                        self.insert_char_raw(ch);
                    }
                }
            }
            EditOperation::Delete { pos, text } => {
                self.cursor = *pos;
                // Move to end of text to delete from correct position
                for _ in 0..text.len() {
                    self.move_cursor_right();
                }
                // Delete characters in reverse to maintain positions
                for _ in 0..text.len() {
                    self.delete_char_raw();
                }
            }
            EditOperation::Replace { pos, old, new } => {
                self.cursor = *pos;
                // Move to end of old text
                for _ in 0..old.len() {
                    self.move_cursor_right();
                }
                // Delete old text first
                for _ in 0..old.len() {
                    self.delete_char_raw();
                }
                // Insert new text
                for ch in new.chars() {
                    if ch == '\n' {
                        self.insert_line_break_raw();
                    } else {
                        self.insert_char_raw(ch);
                    }
                }
            }
        }
    }

    /// Internal method to insert character without saving undo state
    fn insert_char_raw(&mut self, ch: char) {
        if self.cursor.row >= self.lines.len() {
            self.lines.push(String::new());
        }

        let line = &mut self.lines[self.cursor.row];
        if self.cursor.col <= line.len() {
            line.insert(self.cursor.col, ch);
            self.cursor.col += 1;
        }
    }

    /// Internal method to insert line break without saving undo state
    fn insert_line_break_raw(&mut self) {
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
    }

    /// Internal method to delete character without saving undo state
    fn delete_char_raw(&mut self) -> bool {
        if self.cursor.col > 0 {
            let line = &mut self.lines[self.cursor.row];
            if self.cursor.col <= line.len() {
                line.remove(self.cursor.col - 1);
                self.cursor.col -= 1;
                return true;
            }
        } else if self.cursor.row > 0 {
            // Join with previous line
            let current_line = self.lines.remove(self.cursor.row);
            self.cursor.row -= 1;
            self.cursor.col = self.lines[self.cursor.row].len();
            self.lines[self.cursor.row].push_str(&current_line);
            return true;
        }
        false
    }

    /// Move cursor right for position calculation
    fn move_cursor_right(&mut self) {
        if self.cursor.row < self.lines.len() {
            let line = &self.lines[self.cursor.row];
            if self.cursor.col < line.len() {
                self.cursor.col += 1;
            } else if self.cursor.row + 1 < self.lines.len() {
                self.cursor.row += 1;
                self.cursor.col = 0;
            }
        }
    }

    fn create_inverse_operation(&self, operation: &EditOperation) -> EditOperation {
        match operation {
            EditOperation::Insert { pos, text } => EditOperation::Delete {
                pos: *pos,
                text: text.clone(),
            },
            EditOperation::Delete { pos, text } => EditOperation::Insert {
                pos: *pos,
                text: text.clone(),
            },
            EditOperation::Replace { pos, old, new } => EditOperation::Replace {
                pos: *pos,
                old: new.clone(), // What's currently there (new text)
                new: old.clone(), // What we want to restore (old text)
            },
        }
    }

    pub fn undo(&mut self) -> bool {
        debug!(
            "Attempting undo operation (undo stack size: {})",
            self.undo_stack.len()
        );
        if let Some(delta) = self.undo_stack.pop_back() {
            // Save current state to redo stack
            let current_cursor = self.cursor;

            // Apply inverse operations in reverse order
            for operation in delta.operations.iter().rev() {
                let inverse = self.create_inverse_operation(operation);
                self.apply_edit_operation(&inverse);
            }

            // Create redo delta
            let redo_delta = BufferDelta {
                operations: delta.operations,
                cursor_before: current_cursor,
                cursor_after: delta.cursor_before,
            };
            self.redo_stack.push_back(redo_delta);

            // Restore cursor position from before the original operation
            self.cursor = delta.cursor_before;
            self.modified = true;
            debug!(
                "Undo successful, cursor moved to {}:{}",
                self.cursor.row, self.cursor.col
            );
            true
        } else {
            debug!("Undo failed: no states in undo stack");
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        debug!(
            "Attempting redo operation (redo stack size: {})",
            self.redo_stack.len()
        );
        if let Some(delta) = self.redo_stack.pop_back() {
            // Save current state to undo stack
            let current_cursor = self.cursor;

            // Apply original operations
            for operation in &delta.operations {
                self.apply_edit_operation(operation);
            }

            // Create undo delta
            let undo_delta = BufferDelta {
                operations: delta.operations,
                cursor_before: current_cursor,
                cursor_after: delta.cursor_after,
            };
            self.undo_stack.push_back(undo_delta);

            // Set cursor position to after the operation
            self.cursor = delta.cursor_after;
            self.modified = true;
            debug!(
                "Redo successful, cursor moved to {}:{}",
                self.cursor.row, self.cursor.col
            );
            true
        } else {
            debug!("Redo failed: no states in redo stack");
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
            info!("Saving buffer {} to file: {:?}", self.id, path);
            let content = self.lines.join("\n");
            std::fs::write(path, content)?;
            self.modified = false;
            info!("Buffer {} saved successfully", self.id);
        } else {
            warn!("Cannot save buffer {}: no file path set", self.id);
        }
        Ok(())
    }

    /// Delete character at cursor position (like 'x' in Vim)
    pub fn delete_char_at_cursor(&mut self) -> bool {
        trace!(
            "Attempting to delete character at cursor position {}:{}",
            self.cursor.row, self.cursor.col
        );
        if self.cursor.row < self.lines.len() {
            if self.cursor.col < self.lines[self.cursor.row].len() {
                let deleted_char = self.lines[self.cursor.row]
                    .chars()
                    .nth(self.cursor.col)
                    .unwrap_or(' ');
                let operation = EditOperation::Delete {
                    pos: self.cursor,
                    text: deleted_char.to_string(),
                };
                self.save_operation(operation);

                let line = &mut self.lines[self.cursor.row];
                line.remove(self.cursor.col);
                self.modified = true;
                trace!(
                    "Deleted character '{}' at position {}:{}",
                    deleted_char, self.cursor.row, self.cursor.col
                );
                return true;
            }
        }
        false
    }

    /// Delete character before cursor (like 'X' in Vim)
    pub fn delete_char_before_cursor(&mut self) -> bool {
        if self.cursor.col > 0 {
            let deleted_char = self.lines[self.cursor.row]
                .chars()
                .nth(self.cursor.col - 1)
                .unwrap_or(' ');
            let operation = EditOperation::Delete {
                pos: Position {
                    row: self.cursor.row,
                    col: self.cursor.col - 1,
                },
                text: deleted_char.to_string(),
            };
            self.save_operation(operation);

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
            let deleted_line = self.lines[self.cursor.row].clone();
            let operation = EditOperation::Delete {
                pos: Position {
                    row: self.cursor.row,
                    col: 0,
                },
                text: format!("{}\n", deleted_line),
            };
            self.save_operation(operation);

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

    /// Yank (copy) the current line
    pub fn yank_line(&mut self) {
        debug!("Yanking line at row {}", self.cursor.row);
        if self.cursor.row < self.lines.len() {
            let line = &self.lines[self.cursor.row];
            // In vim, yanking a line includes the newline character
            let line_with_newline = format!("{}\n", line);
            self.clipboard = ClipboardContent {
                text: line_with_newline,
                yank_type: YankType::Line,
            };
            debug!("Yanked line: '{}'", line);
        } else {
            warn!(
                "Cannot yank line: cursor row {} out of bounds",
                self.cursor.row
            );
        }
    }

    /// Yank (copy) the current word
    pub fn yank_word(&mut self) {
        if self.cursor.row >= self.lines.len() {
            return;
        }

        let line = &self.lines[self.cursor.row];
        if self.cursor.col >= line.len() {
            return;
        }

        let start_pos = self.cursor.col;
        let mut end_pos = start_pos;

        // Find end of current word
        while end_pos < line.len() && !line.chars().nth(end_pos).unwrap_or(' ').is_whitespace() {
            end_pos += 1;
        }

        if end_pos > start_pos {
            let word = &line[start_pos..end_pos];
            self.clipboard = ClipboardContent {
                text: word.to_string(),
                yank_type: YankType::Character,
            };
        }
    }

    /// Yank (copy) text from current cursor to end of line
    pub fn yank_to_end_of_line(&mut self) {
        if self.cursor.row < self.lines.len() {
            let line = &self.lines[self.cursor.row];
            let text = if self.cursor.col < line.len() {
                &line[self.cursor.col..]
            } else {
                ""
            };

            self.clipboard = ClipboardContent {
                text: text.to_string(),
                yank_type: YankType::Character,
            };
        }
    }

    /// Put (paste) clipboard content after cursor
    pub fn put_after(&mut self) {
        match self.clipboard.yank_type {
            YankType::Line => {
                let operation = EditOperation::Insert {
                    pos: Position {
                        row: self.cursor.row + 1,
                        col: 0,
                    },
                    text: format!("{}\n", self.clipboard.text),
                };
                self.save_operation(operation);

                // Insert new line after current line
                let new_line = self.clipboard.text.clone();
                if self.cursor.row + 1 <= self.lines.len() {
                    self.lines.insert(self.cursor.row + 1, new_line);
                    self.cursor.row += 1;
                    self.cursor.col = 0;
                    self.modified = true;
                }
            }
            YankType::Character => {
                let insert_pos = if self.cursor.col < self.lines[self.cursor.row].len() {
                    self.cursor.col + 1
                } else {
                    self.lines[self.cursor.row].len()
                };
                let operation = EditOperation::Insert {
                    pos: Position {
                        row: self.cursor.row,
                        col: insert_pos,
                    },
                    text: self.clipboard.text.clone(),
                };
                self.save_operation(operation);

                // Insert text after cursor position
                if self.cursor.row < self.lines.len() {
                    let line = &mut self.lines[self.cursor.row];
                    line.insert_str(insert_pos, &self.clipboard.text);
                    self.cursor.col = insert_pos + self.clipboard.text.len() - 1;
                    self.modified = true;
                }
            }
            YankType::Block => {
                // TODO: Implement block paste
                self.put_after_character();
            }
        }
    }

    /// Put (paste) clipboard content before cursor
    pub fn put_before(&mut self) {
        match self.clipboard.yank_type {
            YankType::Line => {
                let operation = EditOperation::Insert {
                    pos: Position {
                        row: self.cursor.row,
                        col: 0,
                    },
                    text: format!("{}\n", self.clipboard.text),
                };
                self.save_operation(operation);

                // Insert new line before current line
                let new_line = self.clipboard.text.clone();
                self.lines.insert(self.cursor.row, new_line);
                self.cursor.col = 0;
                self.modified = true;
            }
            YankType::Character => {
                let operation = EditOperation::Insert {
                    pos: self.cursor,
                    text: self.clipboard.text.clone(),
                };
                self.save_operation(operation);

                // Insert text at cursor position
                if self.cursor.row < self.lines.len() {
                    let line = &mut self.lines[self.cursor.row];
                    line.insert_str(self.cursor.col, &self.clipboard.text);
                    self.cursor.col += self.clipboard.text.len() - 1;
                    self.modified = true;
                }
            }
            YankType::Block => {
                // TODO: Implement block paste
                self.put_before_character();
            }
        }
    }

    /// Helper for character-wise paste after cursor
    fn put_after_character(&mut self) {
        let insert_pos = if self.cursor.col < self.lines[self.cursor.row].len() {
            self.cursor.col + 1
        } else {
            self.lines[self.cursor.row].len()
        };
        let operation = EditOperation::Insert {
            pos: Position {
                row: self.cursor.row,
                col: insert_pos,
            },
            text: self.clipboard.text.clone(),
        };
        self.save_operation(operation);

        if self.cursor.row < self.lines.len() {
            let line = &mut self.lines[self.cursor.row];
            line.insert_str(insert_pos, &self.clipboard.text);
            self.cursor.col = insert_pos + self.clipboard.text.len().saturating_sub(1);
            self.modified = true;
        }
    }

    /// Helper for character-wise paste before cursor
    fn put_before_character(&mut self) {
        let operation = EditOperation::Insert {
            pos: self.cursor,
            text: self.clipboard.text.clone(),
        };
        self.save_operation(operation);

        if self.cursor.row < self.lines.len() {
            let line = &mut self.lines[self.cursor.row];
            line.insert_str(self.cursor.col, &self.clipboard.text);
            self.cursor.col += self.clipboard.text.len().saturating_sub(1);
            self.modified = true;
        }
    }

    /// Delete a range of text with proper undo support
    pub fn delete_range(&mut self, start: Position, end: Position) -> String {
        // Get the text to be deleted
        let deleted_text = self.get_text_in_range(start, end);

        // Create undo operation
        let operation = EditOperation::Delete {
            pos: start,
            text: deleted_text.clone(),
        };
        self.save_operation(operation);

        // Perform the deletion
        if start.row == end.row {
            // Single line deletion
            if let Some(line) = self.lines.get_mut(start.row) {
                let start_col = start.col.min(line.len());
                let end_col = end.col.min(line.len());
                line.drain(start_col..end_col);
            }
        } else {
            // Multi-line deletion
            let start_row = start.row;
            let end_row = end.row.min(self.lines.len().saturating_sub(1));

            // Save the beginning of the first line and end of the last line
            let first_part = if let Some(line) = self.lines.get(start_row) {
                line[..start.col.min(line.len())].to_string()
            } else {
                String::new()
            };

            let last_part = if let Some(line) = self.lines.get(end_row) {
                line[end.col.min(line.len())..].to_string()
            } else {
                String::new()
            };

            // Remove lines
            if end_row >= start_row {
                self.lines.drain(start_row..=end_row);
            }

            // Insert combined line
            let combined = format!("{}{}", first_part, last_part);
            self.lines.insert(start_row, combined);
        }

        // Move cursor to start of deleted range
        self.cursor = start;
        self.modified = true;

        deleted_text
    }

    /// Get text content in a range
    pub fn get_text_in_range(&self, start: Position, end: Position) -> String {
        if start.row == end.row {
            // Single line selection
            if let Some(line) = self.lines.get(start.row) {
                let start_col = start.col.min(line.len());
                let end_col = end.col.min(line.len());
                return line[start_col..end_col].to_string();
            }
        } else {
            // Multi-line selection
            let mut result = String::new();

            // First line (from start_col to end)
            if let Some(line) = self.lines.get(start.row) {
                let start_col = start.col.min(line.len());
                result.push_str(&line[start_col..]);
                result.push('\n');
            }

            // Middle lines (complete lines)
            for row in (start.row + 1)..end.row {
                if let Some(line) = self.lines.get(row) {
                    result.push_str(line);
                    result.push('\n');
                }
            }

            // Last line (from start to end_col)
            if let Some(line) = self.lines.get(end.row) {
                let end_col = end.col.min(line.len());
                result.push_str(&line[..end_col]);
            }

            return result;
        }

        String::new()
    }

    /// Replace text in a range with new text (with undo support)
    pub fn replace_range(&mut self, start: Position, end: Position, new_text: &str) {
        let old_text = self.get_text_in_range(start, end);

        // Create undo operation
        let operation = EditOperation::Replace {
            pos: start,
            old: old_text,
            new: new_text.to_string(),
        };
        self.save_operation(operation);

        // Perform the replacement manually to avoid borrowing issues
        if start.row == end.row {
            // Single line replacement
            if let Some(line) = self.lines.get_mut(start.row) {
                let start_col = start.col.min(line.len());
                let end_col = end.col.min(line.len());
                line.replace_range(start_col..end_col, new_text);
                // Update cursor position
                self.cursor = Position {
                    row: start.row,
                    col: start_col + new_text.len(),
                };
            }
        } else {
            // Multi-line replacement - delete range then insert
            self.delete_range_raw(start, end);
            self.cursor = start;
            for ch in new_text.chars() {
                if ch == '\n' {
                    self.insert_line_break_raw();
                } else {
                    self.insert_char_raw(ch);
                }
            }
        }

        self.modified = true;
    }

    /// Delete range without undo (for internal use)
    fn delete_range_raw(&mut self, start: Position, end: Position) {
        if start.row == end.row {
            // Single line deletion
            if let Some(line) = self.lines.get_mut(start.row) {
                let start_col = start.col.min(line.len());
                let end_col = end.col.min(line.len());
                line.drain(start_col..end_col);
            }
        } else {
            // Multi-line deletion
            let start_row = start.row;
            let end_row = end.row.min(self.lines.len().saturating_sub(1));

            // Save the beginning of the first line and end of the last line
            let first_part = if let Some(line) = self.lines.get(start_row) {
                line[..start.col.min(line.len())].to_string()
            } else {
                String::new()
            };

            let last_part = if let Some(line) = self.lines.get(end_row) {
                line[end.col.min(line.len())..].to_string()
            } else {
                String::new()
            };

            // Remove lines
            if end_row >= start_row {
                self.lines.drain(start_row..=end_row);
            }

            // Insert combined line
            let combined = format!("{}{}", first_part, last_part);
            self.lines.insert(start_row, combined);
        }

        // Move cursor to start of deleted range
        self.cursor = start;
    }

    /// Add indentation to a line
    pub fn indent_line(&mut self, line_num: usize) -> anyhow::Result<()> {
        if line_num < self.lines.len() {
            let operation = EditOperation::Insert {
                pos: Position {
                    row: line_num,
                    col: 0,
                },
                text: "    ".to_string(), // 4 spaces for indentation
            };
            self.save_operation(operation);

            self.lines[line_num].insert_str(0, "    ");
            self.modified = true;
        }
        Ok(())
    }

    /// Remove indentation from a line
    pub fn unindent_line(&mut self, line_num: usize) -> anyhow::Result<()> {
        if line_num < self.lines.len() {
            let line = &self.lines[line_num];
            let chars_to_remove = if line.starts_with("    ") {
                4
            } else if line.starts_with("\t") {
                1
            } else {
                // Count leading spaces up to 4
                line.chars().take(4).take_while(|&c| c == ' ').count()
            };

            if chars_to_remove > 0 {
                let removed_text = self.lines[line_num][..chars_to_remove].to_string();
                let operation = EditOperation::Delete {
                    pos: Position {
                        row: line_num,
                        col: 0,
                    },
                    text: removed_text,
                };
                self.save_operation(operation);

                self.lines[line_num].drain(..chars_to_remove);
                self.modified = true;
            }
        }
        Ok(())
    }
}
