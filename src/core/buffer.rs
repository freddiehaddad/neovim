use crate::core::mode::{Position, Selection, SelectionType};
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

    /// Get the length of a line, returns 0 if the line doesn't exist
    pub fn get_line_length(&self, row: usize) -> usize {
        if row < self.lines.len() {
            self.lines[row].chars().count() // Use UTF-8 safe character count
        } else {
            0
        }
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
        if self.cursor.row < self.lines.len() && self.cursor.col < self.lines[self.cursor.row].len()
        {
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

        pos = pos.saturating_sub(1);

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

        let chars: Vec<char> = line.chars().collect();
        let start_pos = self.cursor.col;
        let mut end_pos = start_pos;

        // Find end of current word
        while end_pos < chars.len() && !chars[end_pos].is_whitespace() {
            end_pos += 1;
        }

        if end_pos > start_pos {
            let word: String = chars[start_pos..end_pos].iter().collect();
            self.clipboard = ClipboardContent {
                text: word,
                yank_type: YankType::Character,
            };
        }
    }

    /// Yank (copy) text from current cursor to end of line
    pub fn yank_to_end_of_line(&mut self) {
        if self.cursor.row < self.lines.len() {
            let line = &self.lines[self.cursor.row];
            let chars: Vec<char> = line.chars().collect();
            let text = if self.cursor.col < chars.len() {
                chars[self.cursor.col..].iter().collect()
            } else {
                String::new()
            };

            self.clipboard = ClipboardContent {
                text,
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
                if self.cursor.row < self.lines.len() {
                    self.lines.insert(self.cursor.row + 1, new_line);
                    self.cursor.row += 1;
                    self.cursor.col = 0;
                    self.modified = true;
                }
            }
            YankType::Character => {
                // Handle multi-line character-wise paste properly
                let clipboard_text = self.clipboard.text.clone();

                // Early return if clipboard is empty
                if clipboard_text.is_empty() {
                    return;
                }

                let text_lines: Vec<&str> = clipboard_text.split('\n').collect();
                let cursor_row = self.cursor.row;
                let cursor_col = self.cursor.col;

                let insert_pos = if cursor_col < self.lines[cursor_row].len() {
                    cursor_col + 1
                } else {
                    self.lines[cursor_row].len()
                };

                let operation = EditOperation::Insert {
                    pos: Position {
                        row: cursor_row,
                        col: insert_pos,
                    },
                    text: clipboard_text.clone(),
                };
                self.save_operation(operation);

                if cursor_row < self.lines.len() && !text_lines.is_empty() {
                    if text_lines.len() == 1 {
                        // Single line - simple case
                        let line = &mut self.lines[cursor_row];
                        line.insert_str(insert_pos, &clipboard_text);
                        self.cursor.col = insert_pos + clipboard_text.len() - 1;
                    } else {
                        // Multi-line - split current line and insert lines between
                        let current_line = self.lines[cursor_row].clone();
                        let (left_part, right_part) = current_line.split_at(insert_pos);

                        // First line: left_part + first_text_line
                        let first_line = format!("{}{}", left_part, text_lines[0]);
                        self.lines[cursor_row] = first_line;

                        // Insert middle lines (if any)
                        for (i, &line_text) in text_lines
                            .iter()
                            .enumerate()
                            .skip(1)
                            .take(text_lines.len().saturating_sub(2))
                        {
                            self.lines.insert(cursor_row + i, line_text.to_string());
                        }

                        // Last line: last_text_line + right_part
                        let last_line = format!("{}{}", text_lines.last().unwrap(), right_part);
                        let insert_row = cursor_row + text_lines.len() - 1;
                        self.lines.insert(insert_row, last_line);

                        // Update cursor to end of pasted text
                        self.cursor.row = insert_row;
                        self.cursor.col = text_lines
                            .last()
                            .map(|line| line.len().saturating_sub(1))
                            .unwrap_or(0);
                    }
                    self.modified = true;
                }
            }
            YankType::Block => {
                self.put_after_block();
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
                // Handle multi-line character-wise paste properly
                let clipboard_text = self.clipboard.text.clone();

                // Early return if clipboard is empty
                if clipboard_text.is_empty() {
                    return;
                }

                let text_lines: Vec<&str> = clipboard_text.split('\n').collect();
                let cursor_row = self.cursor.row;
                let cursor_col = self.cursor.col;

                let operation = EditOperation::Insert {
                    pos: Position {
                        row: cursor_row,
                        col: cursor_col,
                    },
                    text: clipboard_text.clone(),
                };
                self.save_operation(operation);

                if cursor_row < self.lines.len() && !text_lines.is_empty() {
                    if text_lines.len() == 1 {
                        // Single line - simple case
                        let line = &mut self.lines[cursor_row];
                        line.insert_str(cursor_col, &clipboard_text);
                        self.cursor.col += clipboard_text.len() - 1;
                    } else {
                        // Multi-line - split current line and insert lines between
                        let current_line = self.lines[cursor_row].clone();
                        let (left_part, right_part) = current_line.split_at(cursor_col);

                        // First line: left_part + first_text_line
                        let first_line = format!("{}{}", left_part, text_lines[0]);
                        self.lines[cursor_row] = first_line;

                        // Insert middle lines (if any)
                        for (i, &line_text) in text_lines
                            .iter()
                            .enumerate()
                            .skip(1)
                            .take(text_lines.len().saturating_sub(2))
                        {
                            self.lines.insert(cursor_row + i, line_text.to_string());
                        }

                        // Last line: last_text_line + right_part
                        let last_line = format!("{}{}", text_lines.last().unwrap(), right_part);
                        let insert_row = cursor_row + text_lines.len() - 1;
                        self.lines.insert(insert_row, last_line);

                        // Update cursor to end of pasted text
                        self.cursor.row = insert_row;
                        self.cursor.col = text_lines
                            .last()
                            .map(|line| line.len().saturating_sub(1))
                            .unwrap_or(0);
                    }
                    self.modified = true;
                }
            }
            YankType::Block => {
                self.put_before_block();
            }
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
                let chars: Vec<char> = line.chars().collect();
                let start_col = start.col.min(chars.len());
                let end_col = end.col.min(chars.len());

                // Rebuild the line without the deleted characters
                let before: String = chars[..start_col].iter().collect();
                let after: String = chars[end_col..].iter().collect();
                *line = format!("{}{}", before, after);
            }
        } else {
            // Multi-line deletion
            let start_row = start.row;
            let end_row = end.row.min(self.lines.len().saturating_sub(1));

            // Save the beginning of the first line and end of the last line
            let first_part = if let Some(line) = self.lines.get(start_row) {
                let chars: Vec<char> = line.chars().collect();
                let start_col = start.col.min(chars.len());
                chars[..start_col].iter().collect()
            } else {
                String::new()
            };

            let last_part = if let Some(line) = self.lines.get(end_row) {
                let chars: Vec<char> = line.chars().collect();
                let end_col = end.col.min(chars.len());
                chars[end_col..].iter().collect()
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
                let chars: Vec<char> = line.chars().collect();
                let start_col = start.col.min(chars.len());
                let end_col = end.col.min(chars.len());
                return chars[start_col..end_col].iter().collect();
            }
        } else {
            // Multi-line selection
            let mut result = String::new();

            // First line (from start_col to end)
            if let Some(line) = self.lines.get(start.row) {
                let chars: Vec<char> = line.chars().collect();
                let start_col = start.col.min(chars.len());
                let selected: String = chars[start_col..].iter().collect();
                result.push_str(&selected);
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
                let chars: Vec<char> = line.chars().collect();
                let end_col = end.col.min(chars.len());
                let selected: String = chars[..end_col].iter().collect();
                result.push_str(&selected);
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
                let chars: Vec<char> = line.chars().collect();
                let start_col = start.col.min(chars.len());
                let end_col = end.col.min(chars.len());

                // Rebuild the line without the deleted characters
                let before: String = chars[..start_col].iter().collect();
                let after: String = chars[end_col..].iter().collect();
                *line = format!("{}{}", before, after);
            }
        } else {
            // Multi-line deletion
            let start_row = start.row;
            let end_row = end.row.min(self.lines.len().saturating_sub(1));

            // Save the beginning of the first line and end of the last line
            let first_part = if let Some(line) = self.lines.get(start_row) {
                let chars: Vec<char> = line.chars().collect();
                let start_col = start.col.min(chars.len());
                chars[..start_col].iter().collect()
            } else {
                String::new()
            };

            let last_part = if let Some(line) = self.lines.get(end_row) {
                let chars: Vec<char> = line.chars().collect();
                let end_col = end.col.min(chars.len());
                chars[end_col..].iter().collect()
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
                let chars: Vec<char> = line.chars().collect();
                let removed_text: String = chars[..chars_to_remove].iter().collect();
                let operation = EditOperation::Delete {
                    pos: Position {
                        row: line_num,
                        col: 0,
                    },
                    text: removed_text,
                };
                self.save_operation(operation);

                let remaining: String = chars[chars_to_remove..].iter().collect();
                self.lines[line_num] = remaining;
                self.modified = true;
            }
        }
        Ok(())
    }

    // ===== Visual Selection Methods =====

    /// Start visual selection at current cursor position
    pub fn start_visual_selection(&mut self) {
        debug!(
            "Starting character-wise visual selection at position {:?}",
            self.cursor
        );
        self.selection = Some(Selection::new(self.cursor, self.cursor));
    }

    /// Start visual line selection at current cursor position  
    pub fn start_visual_line_selection(&mut self) {
        debug!(
            "Starting line-wise visual selection at row {}",
            self.cursor.row
        );
        // For line-wise selection, we select entire lines
        let start_pos = Position::new(self.cursor.row, 0);
        let end_pos = Position::new(self.cursor.row, self.get_line_length(self.cursor.row));
        self.selection = Some(Selection::new_line(start_pos, end_pos));
    }

    /// Start visual block selection at current cursor position
    pub fn start_visual_block_selection(&mut self) {
        debug!(
            "Starting block-wise visual selection at position {:?}",
            self.cursor
        );
        // For block-wise selection, start with a 1x1 block at cursor position
        self.selection = Some(Selection::new_with_type(
            self.cursor,
            self.cursor,
            SelectionType::Block,
        ));
    }

    /// Update visual selection end position as cursor moves
    pub fn update_visual_selection(&mut self, end_pos: Position) {
        if let Some(selection) = &mut self.selection {
            debug!(
                "Updating visual selection end to {:?}, selection_type: {:?}",
                end_pos, selection.selection_type
            );
            match selection.selection_type {
                SelectionType::Character => {
                    // Character-wise: update end position directly
                    selection.end = end_pos;
                }
                SelectionType::Line => {
                    // Line-wise: extend selection to include entire lines
                    let start_row = selection.start.row.min(end_pos.row);
                    let end_row = selection.start.row.max(end_pos.row);

                    // Get line length before borrowing mutably
                    let end_line_length = if end_row < self.lines.len() {
                        self.lines[end_row].chars().count()
                    } else {
                        0
                    };

                    selection.start = Position::new(start_row, 0);
                    selection.end = Position::new(end_row, end_line_length);

                    debug!(
                        "Updated line-wise selection: rows {} to {}",
                        start_row, end_row
                    );
                }
                SelectionType::Block => {
                    // Block-wise: create rectangular selection
                    debug!(
                        "Updating block-wise selection from {:?} to {:?}",
                        selection.start, end_pos
                    );
                    selection.end = end_pos;
                }
            }
        }
    }

    /// Clear visual selection
    pub fn clear_visual_selection(&mut self) {
        if self.selection.is_some() {
            debug!("Clearing visual selection");
            self.selection = None;
        }
    }

    /// Get the current visual selection range (normalized)
    /// Returns (start, end) where start is always before end in document order
    pub fn get_selection_range(&self) -> Option<(Position, Position)> {
        self.selection.map(|sel| {
            let start = sel.start;
            let end = sel.end;

            // Normalize the selection so start is before end
            if start.row < end.row || (start.row == end.row && start.col <= end.col) {
                (start, end)
            } else {
                (end, start)
            }
        })
    }

    /// Get the current visual selection with type information
    /// Returns the Selection struct which includes the selection type
    pub fn get_selection(&self) -> Option<Selection> {
        self.selection
    }

    /// Get text content of current visual selection
    pub fn get_selected_text(&self) -> Option<String> {
        if let Some(selection) = self.selection {
            use crate::core::mode::SelectionType;

            match selection.selection_type {
                SelectionType::Character | SelectionType::Line => {
                    // For character and line selections, use the existing logic
                    if let Some((start, end)) = self.get_selection_range() {
                        Some(self.get_text_in_range(start, end))
                    } else {
                        None
                    }
                }
                SelectionType::Block => {
                    // For block selection, extract rectangular text region
                    self.get_block_selected_text(selection)
                }
            }
        } else {
            None
        }
    }

    /// Get text content for block selection (rectangular region)
    fn get_block_selected_text(&self, selection: Selection) -> Option<String> {
        let (start, end) = if selection.start.row <= selection.end.row {
            (selection.start, selection.end)
        } else {
            (selection.end, selection.start)
        };

        let left_col = start.col.min(end.col);
        let right_col = start.col.max(end.col) + 1; // +1 to make it inclusive

        let mut result = Vec::new();

        for row in start.row..=end.row {
            if row < self.lines.len() {
                let line = &self.lines[row];
                let line_chars: Vec<char> = line.chars().collect();

                // Extract the rectangular region from this line
                let start_col = left_col.min(line_chars.len());
                let end_col = right_col.min(line_chars.len()).max(start_col);

                let line_segment: String = if start_col < line_chars.len() {
                    // Extract characters from line, pad with spaces if selection extends beyond line
                    let extracted: String = line_chars[start_col..end_col].iter().collect();
                    let width = right_col.saturating_sub(left_col);
                    if extracted.len() < width {
                        format!("{}{}", extracted, " ".repeat(width - extracted.len()))
                    } else {
                        extracted
                    }
                } else {
                    // Line is shorter than selection start, add spaces to maintain block structure
                    " ".repeat(right_col.saturating_sub(left_col))
                };

                result.push(line_segment);

                debug!(
                    "Block selection row {}: cols {}..{} -> '{}'",
                    row,
                    left_col,
                    right_col,
                    result.last().unwrap_or(&String::new())
                );
            } else {
                // Beyond file end, add empty line with appropriate spacing
                result.push(" ".repeat(right_col.saturating_sub(left_col)));
            }
        }

        if result.is_empty() {
            None
        } else {
            let block_text = result.join("\n");
            debug!("Block selection result: {} lines", result.len());
            Some(block_text)
        }
    }

    /// Delete the currently selected text (visual mode delete)
    pub fn delete_selection(&mut self) -> Option<String> {
        if let Some((start, end)) = self.get_selection_range() {
            let deleted_text = self.delete_range(start, end);
            self.selection = None;
            debug!("Deleted visual selection: {} chars", deleted_text.len());
            Some(deleted_text)
        } else {
            None
        }
    }

    /// Yank (copy) the currently selected text
    pub fn yank_selection(&mut self) -> Option<String> {
        if let Some(selected_text) = self.get_selected_text() {
            let yank_type = if let Some(selection) = self.selection {
                match selection.selection_type {
                    SelectionType::Character => YankType::Character,
                    SelectionType::Line => YankType::Line,
                    SelectionType::Block => YankType::Block,
                }
            } else {
                YankType::Character
            };

            self.clipboard = ClipboardContent {
                text: selected_text.clone(),
                yank_type,
            };
            // Clear the selection after yanking (matches Vim behavior)
            self.selection = None;
            debug!(
                "Yanked visual selection: {} chars, type: {:?}",
                selected_text.len(),
                self.clipboard.yank_type
            );
            Some(selected_text)
        } else {
            None
        }
    }

    /// Check if there is an active visual selection
    pub fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    /// Helper for block-wise paste after cursor
    fn put_after_block(&mut self) {
        let text = self.clipboard.text.clone();
        if text.is_empty() {
            return;
        }

        let lines: Vec<&str> = text.split('\n').collect();

        // Special handling for buffer extension: if cursor is on the last line,
        // paste starting from the next row at column 0
        let (paste_row, paste_col) = if self.cursor.row == self.lines.len() - 1 {
            // Cursor is on the last line - extend buffer with new lines
            (self.cursor.row + 1, 0)
        } else {
            // Cursor is not on the last line - paste within existing lines
            (self.cursor.row, self.cursor.col + 1)
        };

        debug!(
            "Block paste after cursor: {} lines at row {}, col {} (cursor was at {}, {})",
            lines.len(),
            paste_row,
            paste_col,
            self.cursor.row,
            self.cursor.col
        );

        self.insert_block_text(&lines, paste_row, paste_col);
    }

    /// Helper for block-wise paste before cursor
    fn put_before_block(&mut self) {
        let text = self.clipboard.text.clone();
        if text.is_empty() {
            return;
        }

        let lines: Vec<&str> = text.split('\n').collect();
        let paste_row = self.cursor.row;
        let paste_col = self.cursor.col; // Before cursor

        debug!(
            "Block paste before cursor: {} lines at row {}, col {}",
            lines.len(),
            paste_row,
            paste_col
        );

        self.insert_block_text(&lines, paste_row, paste_col);
    }

    /// Insert block text at specified position
    fn insert_block_text(&mut self, lines: &[&str], start_row: usize, start_col: usize) {
        // Ensure we have enough lines in the buffer
        while self.lines.len() < start_row + lines.len() {
            self.lines.push(String::new());
        }

        for (i, line_text) in lines.iter().enumerate() {
            let target_row = start_row + i;
            if target_row < self.lines.len() {
                let target_line = &mut self.lines[target_row];
                let mut chars: Vec<char> = target_line.chars().collect();

                // Only extend line with spaces if start_col is beyond the line length
                if start_col > chars.len() {
                    let spaces_needed = start_col - chars.len();
                    chars.extend(std::iter::repeat_n(' ', spaces_needed));
                }

                // Insert block text at the specified column
                let insert_text = line_text.to_string();

                // Insert the text at the specified column position
                if start_col <= chars.len() {
                    // Split the line and insert the block text
                    let before: String = chars[..start_col].iter().collect();
                    let after: String = chars[start_col..].iter().collect();
                    *target_line = format!("{}{}{}", before, insert_text, after);
                } else {
                    // This case should not happen since we extend the line above
                    let spaces_needed = start_col - chars.len();
                    let spaces = " ".repeat(spaces_needed);
                    *target_line = format!("{}{}{}", target_line, spaces, insert_text);
                }
                debug!(
                    "Block paste row {}: inserted '{}' at col {}",
                    target_row, line_text, start_col
                );
            }
        }

        // Update cursor position to the top-left of pasted block
        self.cursor.row = start_row;
        self.cursor.col = start_col
            + if !lines.is_empty() && !lines[0].is_empty() {
                lines[0].len().saturating_sub(1)
            } else {
                0
            };
        self.modified = true;
    }
}
