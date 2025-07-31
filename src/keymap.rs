use crate::editor::Editor;
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeymapConfig {
    pub normal_mode: HashMap<String, String>,
    pub insert_mode: HashMap<String, String>,
    pub command_mode: HashMap<String, String>,
    pub visual_mode: HashMap<String, String>,
    pub replace_mode: HashMap<String, String>,
    pub search_mode: HashMap<String, String>,
}

#[derive(Clone)]
pub struct KeyHandler {
    keymap_config: KeymapConfig,
    pending_sequence: String,
    last_key_time: Option<std::time::Instant>,
}

impl KeyHandler {
    pub fn new() -> Self {
        Self {
            keymap_config: Self::load_default_keymaps(),
            pending_sequence: String::new(),
            last_key_time: None,
        }
    }

    fn load_default_keymaps() -> KeymapConfig {
        // Try to load from keymaps.toml file, fall back to defaults if not found
        if let Ok(config_content) = fs::read_to_string("keymaps.toml") {
            if let Ok(config) = toml::from_str(&config_content) {
                return config;
            }
        }

        // Fallback to minimal default keymaps
        Self::create_default_keymaps()
    }

    fn create_default_keymaps() -> KeymapConfig {
        let mut normal_mode = HashMap::new();
        normal_mode.insert("h".to_string(), "cursor_left".to_string());
        normal_mode.insert("j".to_string(), "cursor_down".to_string());
        normal_mode.insert("k".to_string(), "cursor_up".to_string());
        normal_mode.insert("l".to_string(), "cursor_right".to_string());
        normal_mode.insert("i".to_string(), "insert_mode".to_string());
        normal_mode.insert(":".to_string(), "command_mode".to_string());
        normal_mode.insert("/".to_string(), "search_forward".to_string());

        let mut insert_mode = HashMap::new();
        insert_mode.insert("Escape".to_string(), "normal_mode".to_string());
        insert_mode.insert("Char".to_string(), "insert_char".to_string());
        insert_mode.insert("Enter".to_string(), "new_line".to_string());
        insert_mode.insert("Backspace".to_string(), "delete_char".to_string());

        let mut command_mode = HashMap::new();
        command_mode.insert("Escape".to_string(), "normal_mode".to_string());
        command_mode.insert("Enter".to_string(), "execute_command".to_string());
        command_mode.insert("Char".to_string(), "append_command".to_string());
        command_mode.insert("Backspace".to_string(), "delete_command_char".to_string());

        let mut search_mode = HashMap::new();
        search_mode.insert("Escape".to_string(), "normal_mode".to_string());
        search_mode.insert("Enter".to_string(), "execute_search".to_string());
        search_mode.insert("Char".to_string(), "append_search".to_string());
        search_mode.insert("Backspace".to_string(), "delete_search_char".to_string());

        KeymapConfig {
            normal_mode,
            insert_mode,
            command_mode,
            visual_mode: HashMap::new(),
            replace_mode: HashMap::new(),
            search_mode,
        }
    }

    pub fn handle_key(&mut self, editor: &mut Editor, key: KeyEvent) -> Result<()> {
        let key_string = Self::key_event_to_string(key);

        // Handle key sequences for normal mode
        if matches!(editor.mode(), Mode::Normal) {
            // Check for timeout (reset sequence if too much time passed)
            let now = Instant::now();
            if let Some(last_time) = self.last_key_time {
                if now.duration_since(last_time).as_millis() > 1000 {
                    self.pending_sequence.clear();
                }
            }
            self.last_key_time = Some(now);

            // Add current key to sequence
            if !self.pending_sequence.is_empty() {
                self.pending_sequence.push_str(&key_string);
            } else {
                self.pending_sequence = key_string.clone();
            }

            // Check if sequence matches any command
            if let Some(action) = self.keymap_config.normal_mode.get(&self.pending_sequence) {
                let action_result = self.execute_action(editor, action, key);
                self.pending_sequence.clear();
                return action_result;
            }

            // Check if pending sequence could be part of a longer command
            let has_potential_match = self
                .keymap_config
                .normal_mode
                .keys()
                .any(|k| k.starts_with(&self.pending_sequence) && k != &self.pending_sequence);

            if !has_potential_match {
                // No potential matches, try single key
                if let Some(action) = self.keymap_config.normal_mode.get(&key_string) {
                    let action_result = self.execute_action(editor, action, key);
                    self.pending_sequence.clear();
                    return action_result;
                }
                self.pending_sequence.clear();
            }

            return Ok(());
        }

        let action = match editor.mode() {
            Mode::Normal => self.keymap_config.normal_mode.get(&key_string),
            Mode::Insert => {
                if let KeyCode::Char(_) = key.code {
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT)
                    {
                        self.keymap_config.insert_mode.get("Char")
                    } else {
                        self.keymap_config.insert_mode.get(&key_string)
                    }
                } else {
                    self.keymap_config.insert_mode.get(&key_string)
                }
            }
            Mode::Command => {
                if let KeyCode::Char(_) = key.code {
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT)
                    {
                        self.keymap_config.command_mode.get("Char")
                    } else {
                        self.keymap_config.command_mode.get(&key_string)
                    }
                } else {
                    self.keymap_config.command_mode.get(&key_string)
                }
            }
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => {
                self.keymap_config.visual_mode.get(&key_string)
            }
            Mode::Replace => {
                if let KeyCode::Char(_) = key.code {
                    self.keymap_config.replace_mode.get("Char")
                } else {
                    self.keymap_config.replace_mode.get(&key_string)
                }
            }
            Mode::Search => {
                if let KeyCode::Char(_) = key.code {
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT)
                    {
                        self.keymap_config.search_mode.get("Char")
                    } else {
                        self.keymap_config.search_mode.get(&key_string)
                    }
                } else {
                    self.keymap_config.search_mode.get(&key_string)
                }
            }
        };

        if let Some(action_name) = action {
            self.execute_action(editor, action_name, key)?;
        }

        Ok(())
    }

    fn key_event_to_string(key: KeyEvent) -> String {
        match key.code {
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Left => "Left".to_string(),
            KeyCode::Right => "Right".to_string(),
            KeyCode::Up => "Up".to_string(),
            KeyCode::Down => "Down".to_string(),
            KeyCode::Backspace => "Backspace".to_string(),
            KeyCode::Esc => "Escape".to_string(),
            _ => "Unknown".to_string(),
        }
    }

    fn execute_action(&self, editor: &mut Editor, action: &str, key: KeyEvent) -> Result<()> {
        match action {
            // Movement actions
            "cursor_left" => self.action_cursor_left(editor)?,
            "cursor_right" => self.action_cursor_right(editor)?,
            "cursor_up" => self.action_cursor_up(editor)?,
            "cursor_down" => self.action_cursor_down(editor)?,

            // Word movement
            "word_forward" => self.action_word_forward(editor)?,
            "word_backward" => self.action_word_backward(editor)?,
            "word_end" => self.action_word_end(editor)?,

            // Delete operations
            "delete_char_at_cursor" => self.action_delete_char_at_cursor(editor)?,
            "delete_char_before_cursor" => self.action_delete_char_before_cursor(editor)?,
            "delete_line" => self.action_delete_line(editor)?,

            // Yank (copy) operations
            "yank_line" => self.action_yank_line(editor)?,
            "yank_word" => self.action_yank_word(editor)?,
            "yank_to_end_of_line" => self.action_yank_to_end_of_line(editor)?,

            // Put (paste) operations
            "put_after" => self.action_put_after(editor)?,
            "put_before" => self.action_put_before(editor)?,

            // Line movement
            "line_start" => self.action_line_start(editor)?,
            "line_end" => self.action_line_end(editor)?,
            "line_first_char" => self.action_line_start(editor)?, // Temporary fallback

            // Buffer movement
            "buffer_start" => self.action_buffer_start(editor)?,
            "buffer_end" => self.action_buffer_end(editor)?,

            // Mode transitions
            "insert_mode" => self.action_insert_mode(editor)?,
            "insert_line_start" => self.action_insert_line_start(editor)?,
            "insert_after" => self.action_insert_after(editor)?,
            "insert_line_end" => self.action_insert_line_end(editor)?,
            "insert_line_below" => self.action_insert_line_below(editor)?,
            "insert_line_above" => self.action_insert_line_above(editor)?,
            "normal_mode" => self.action_normal_mode(editor)?,
            "command_mode" => self.action_command_mode(editor)?,
            "visual_mode" => self.action_visual_mode(editor)?,
            "visual_line_mode" => self.action_visual_line_mode(editor)?,
            "visual_block_mode" => self.action_visual_block_mode(editor)?,
            "replace_mode" => self.action_replace_mode(editor)?,
            "search_forward" => self.action_search_forward(editor)?,
            "search_backward" => self.action_search_backward(editor)?,
            "search_next" => self.action_search_next(editor)?,
            "search_previous" => self.action_search_previous(editor)?,

            // File operations
            "save_file" => self.action_save_file(editor)?,
            "quit" => self.action_quit(editor)?,

            // Undo/Redo
            "undo" => self.action_undo(editor)?,
            "redo" => self.action_redo(editor)?,

            // Insert mode actions
            "insert_char" => self.action_insert_char(editor, key)?,
            "new_line" => self.action_new_line(editor)?,
            "delete_char" => self.action_delete_char(editor)?,
            "delete_char_forward" => self.action_delete_char_forward(editor)?,
            "insert_tab" => self.action_insert_tab(editor)?,

            // Command mode actions
            "append_command" => self.action_append_command(editor, key)?,
            "delete_command_char" => self.action_delete_command_char(editor)?,
            "execute_command" => self.action_execute_command(editor)?,

            // Search mode actions
            "append_search" => self.action_append_search(editor, key)?,
            "delete_search_char" => self.action_delete_search_char(editor)?,
            "execute_search" => self.action_execute_search(editor)?,

            // Visual mode actions
            "delete_selection" => self.action_delete_selection(editor)?,
            "yank_selection" => self.action_yank_selection(editor)?,
            "change_selection" => self.action_change_selection(editor)?,

            // Replace mode actions
            "replace_char" => self.action_replace_char(editor, key)?,

            _ => return Ok(()), // Unknown action, ignore
        }
        Ok(())
    }

    // Action implementations
    fn action_cursor_left(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            if buffer.cursor.col > 0 {
                buffer.cursor.col -= 1;
            }
        }
        Ok(())
    }

    fn action_cursor_right(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            if let Some(line) = buffer.get_line(buffer.cursor.row) {
                if buffer.cursor.col < line.len() {
                    buffer.cursor.col += 1;
                }
            }
        }
        Ok(())
    }

    fn action_cursor_up(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            if buffer.cursor.row > 0 {
                buffer.cursor.row -= 1;
                if let Some(line) = buffer.get_line(buffer.cursor.row) {
                    buffer.cursor.col = buffer.cursor.col.min(line.len());
                }
            }
        }
        Ok(())
    }

    fn action_cursor_down(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            if buffer.cursor.row < buffer.lines.len() - 1 {
                buffer.cursor.row += 1;
                if let Some(line) = buffer.get_line(buffer.cursor.row) {
                    buffer.cursor.col = buffer.cursor.col.min(line.len());
                }
            }
        }
        Ok(())
    }

    fn action_insert_mode(&self, editor: &mut Editor) -> Result<()> {
        editor.set_mode(Mode::Insert);
        Ok(())
    }

    fn action_normal_mode(&self, editor: &mut Editor) -> Result<()> {
        editor.set_mode(Mode::Normal);
        Ok(())
    }

    fn action_command_mode(&self, editor: &mut Editor) -> Result<()> {
        editor.set_mode(Mode::Command);
        editor.set_command_line(":".to_string());
        Ok(())
    }

    fn action_search_forward(&self, editor: &mut Editor) -> Result<()> {
        editor.set_mode(Mode::Search);
        editor.set_command_line("/".to_string());
        Ok(())
    }

    fn action_insert_char(&self, editor: &mut Editor, key: KeyEvent) -> Result<()> {
        if let KeyCode::Char(ch) = key.code {
            if let Some(buffer) = editor.current_buffer_mut() {
                buffer.insert_char(ch);
            }
        }
        Ok(())
    }

    fn action_new_line(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.insert_line_break();
        }
        Ok(())
    }

    fn action_delete_char(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.delete_char();
        }
        Ok(())
    }

    fn action_append_command(&self, editor: &mut Editor, key: KeyEvent) -> Result<()> {
        if let KeyCode::Char(ch) = key.code {
            let mut command = editor.command_line().to_string();
            command.push(ch);
            editor.set_command_line(command);
        }
        Ok(())
    }

    fn action_delete_command_char(&self, editor: &mut Editor) -> Result<()> {
        let mut command = editor.command_line().to_string();
        if command.len() > 1 {
            command.pop();
            editor.set_command_line(command);
        }
        Ok(())
    }

    fn action_execute_command(&self, editor: &mut Editor) -> Result<()> {
        let command = editor.command_line().trim_start_matches(':').to_string();

        match command.as_str() {
            "q" | "quit" => editor.quit(),
            "q!" | "quit!" => editor.force_quit(),
            "w" | "write" => {
                if let Some(buffer) = editor.current_buffer_mut() {
                    match buffer.save() {
                        Ok(_) => editor.set_status_message("File saved".to_string()),
                        Err(e) => editor.set_status_message(format!("Error saving: {}", e)),
                    }
                }
            }
            "wq" | "x" => {
                if let Some(buffer) = editor.current_buffer_mut() {
                    match buffer.save() {
                        Ok(_) => {
                            editor.set_status_message("File saved".to_string());
                            editor.quit();
                        }
                        Err(e) => editor.set_status_message(format!("Error saving: {}", e)),
                    }
                } else {
                    editor.quit();
                }
            }
            // Line number commands
            "set nu" | "set number" => editor.set_line_numbers(true, false),
            "set nonu" | "set nonumber" => editor.set_line_numbers(false, false),
            "set rnu" | "set relativenumber" => editor.set_line_numbers(false, true),
            "set nornu" | "set norelativenumber" => editor.set_line_numbers(true, false),
            "set nu rnu" | "set number relativenumber" => editor.set_line_numbers(true, true),
            // Cursor line commands
            "set cul" | "set cursorline" => editor.set_cursor_line(true),
            "set nocul" | "set nocursorline" => editor.set_cursor_line(false),
            _ => {
                // Try to handle generic :set commands
                if command.starts_with("set ") {
                    self.handle_set_command(editor, &command[4..]);
                } else {
                    editor.set_status_message(format!("Unknown command: {}", command));
                }
            }
        }

        editor.set_mode(Mode::Normal);
        editor.set_command_line(String::new());
        Ok(())
    }

    fn action_append_search(&self, editor: &mut Editor, key: KeyEvent) -> Result<()> {
        if let KeyCode::Char(ch) = key.code {
            let mut search = editor.command_line().to_string();
            search.push(ch);
            editor.set_command_line(search);
        }
        Ok(())
    }

    fn action_delete_search_char(&self, editor: &mut Editor) -> Result<()> {
        let mut search = editor.command_line().to_string();
        if search.len() > 1 {
            search.pop();
            editor.set_command_line(search);
        }
        Ok(())
    }

    fn action_execute_search(&self, editor: &mut Editor) -> Result<()> {
        let search_term = editor.command_line()[1..].to_string();
        if !search_term.is_empty() {
            editor.search(&search_term);
        }
        editor.set_mode(Mode::Normal);
        editor.set_command_line(String::new());
        Ok(())
    }

    // Additional action implementations
    fn action_line_start(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.cursor.col = 0;
        }
        Ok(())
    }

    fn action_line_end(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            if let Some(line) = buffer.get_line(buffer.cursor.row) {
                buffer.cursor.col = line.len();
            }
        }
        Ok(())
    }

    fn action_buffer_start(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.cursor.row = 0;
            buffer.cursor.col = 0;
        }
        Ok(())
    }

    fn action_buffer_end(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.cursor.row = buffer.lines.len().saturating_sub(1);
            if let Some(line) = buffer.get_line(buffer.cursor.row) {
                buffer.cursor.col = line.len();
            }
        }
        Ok(())
    }

    fn action_insert_line_start(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.cursor.col = 0;
        }
        editor.set_mode(Mode::Insert);
        Ok(())
    }

    fn action_insert_after(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            if let Some(line) = buffer.get_line(buffer.cursor.row) {
                if buffer.cursor.col < line.len() {
                    buffer.cursor.col += 1;
                }
            }
        }
        editor.set_mode(Mode::Insert);
        Ok(())
    }

    fn action_insert_line_end(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            if let Some(line) = buffer.get_line(buffer.cursor.row) {
                buffer.cursor.col = line.len();
            }
        }
        editor.set_mode(Mode::Insert);
        Ok(())
    }

    fn action_insert_line_below(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            let row = buffer.cursor.row;
            buffer.lines.insert(row + 1, String::new());
            buffer.cursor.row = row + 1;
            buffer.cursor.col = 0;
            buffer.modified = true;
        }
        editor.set_mode(Mode::Insert);
        Ok(())
    }

    fn action_insert_line_above(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            let row = buffer.cursor.row;
            buffer.lines.insert(row, String::new());
            buffer.cursor.col = 0;
            buffer.modified = true;
        }
        editor.set_mode(Mode::Insert);
        Ok(())
    }

    fn action_visual_mode(&self, editor: &mut Editor) -> Result<()> {
        editor.set_mode(Mode::Visual);
        Ok(())
    }

    fn action_visual_line_mode(&self, editor: &mut Editor) -> Result<()> {
        editor.set_mode(Mode::VisualLine);
        Ok(())
    }

    fn action_visual_block_mode(&self, editor: &mut Editor) -> Result<()> {
        editor.set_mode(Mode::VisualBlock);
        Ok(())
    }

    fn action_replace_mode(&self, editor: &mut Editor) -> Result<()> {
        editor.set_mode(Mode::Replace);
        Ok(())
    }

    fn action_search_backward(&self, editor: &mut Editor) -> Result<()> {
        editor.set_mode(Mode::Search);
        editor.set_command_line("?".to_string());
        Ok(())
    }

    fn action_save_file(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            match buffer.save() {
                Ok(_) => editor.set_status_message("File saved".to_string()),
                Err(e) => editor.set_status_message(format!("Error saving: {}", e)),
            }
        }
        Ok(())
    }

    fn action_quit(&self, editor: &mut Editor) -> Result<()> {
        editor.quit();
        Ok(())
    }

    fn action_undo(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.undo();
        }
        Ok(())
    }

    fn action_redo(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.redo();
        }
        Ok(())
    }

    fn action_delete_char_forward(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            if let Some(line) = buffer.lines.get_mut(buffer.cursor.row) {
                if buffer.cursor.col < line.len() {
                    line.remove(buffer.cursor.col);
                    buffer.modified = true;
                }
            }
        }
        Ok(())
    }

    fn action_insert_tab(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.insert_char('\t');
        }
        Ok(())
    }

    fn action_replace_char(&self, editor: &mut Editor, key: KeyEvent) -> Result<()> {
        if let KeyCode::Char(ch) = key.code {
            if let Some(buffer) = editor.current_buffer_mut() {
                if let Some(line) = buffer.lines.get_mut(buffer.cursor.row) {
                    if buffer.cursor.col < line.len() {
                        line.replace_range(
                            buffer.cursor.col..buffer.cursor.col + 1,
                            &ch.to_string(),
                        );
                        if buffer.cursor.col < line.len() {
                            buffer.cursor.col += 1;
                        }
                        buffer.modified = true;
                    }
                }
            }
        }
        Ok(())
    }

    fn action_delete_selection(&self, editor: &mut Editor) -> Result<()> {
        editor.set_status_message("Delete selection not implemented".to_string());
        Ok(())
    }

    fn action_yank_selection(&self, editor: &mut Editor) -> Result<()> {
        editor.set_status_message("Yank selection not implemented".to_string());
        Ok(())
    }

    fn action_change_selection(&self, editor: &mut Editor) -> Result<()> {
        editor.set_status_message("Change selection not implemented".to_string());
        Ok(())
    }

    fn action_word_forward(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.move_to_next_word();
        }
        Ok(())
    }

    fn action_word_backward(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.move_to_previous_word();
        }
        Ok(())
    }

    fn action_word_end(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.move_to_word_end();
        }
        Ok(())
    }

    fn action_delete_char_at_cursor(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.delete_char_at_cursor();
        }
        Ok(())
    }

    fn action_delete_char_before_cursor(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.delete_char_before_cursor();
        }
        Ok(())
    }

    fn action_delete_line(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.delete_line();
        }
        Ok(())
    }

    fn action_yank_line(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.yank_line();
            editor.set_status_message("Line yanked".to_string());
        }
        Ok(())
    }

    fn action_yank_word(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.yank_word();
            editor.set_status_message("Word yanked".to_string());
        }
        Ok(())
    }

    fn action_yank_to_end_of_line(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.yank_to_end_of_line();
            editor.set_status_message("Text yanked to end of line".to_string());
        }
        Ok(())
    }

    fn action_put_after(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.put_after();
            editor.set_status_message("Text pasted after cursor".to_string());
        }
        Ok(())
    }

    fn action_put_before(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            buffer.put_before();
            editor.set_status_message("Text pasted before cursor".to_string());
        }
        Ok(())
    }

    fn action_search_next(&self, editor: &mut Editor) -> Result<()> {
        editor.search_next();
        Ok(())
    }

    fn action_search_previous(&self, editor: &mut Editor) -> Result<()> {
        editor.search_previous();
        Ok(())
    }

    /// Handle generic :set commands
    fn handle_set_command(&self, editor: &mut Editor, args: &str) {
        let args = args.trim();

        // Handle "no" prefix for disabling options
        if let Some(setting) = args.strip_prefix("no") {
            match setting {
                "number" | "nu" => {
                    editor.set_config_setting("number", "false");
                    let (_, relative) = editor.get_line_number_state();
                    editor.set_line_numbers(false, relative);
                }
                "relativenumber" | "rnu" => {
                    editor.set_config_setting("relativenumber", "false");
                    let (absolute, _) = editor.get_line_number_state();
                    editor.set_line_numbers(absolute, false);
                }
                "cursorline" | "cul" => {
                    editor.set_config_setting("cursorline", "false");
                    editor.set_cursor_line(false);
                }
                _ => editor.set_status_message(format!("Unknown option: no{}", setting)),
            }
            return;
        }

        // Handle setting with values (e.g., "tabstop=4")
        if let Some((setting, value)) = args.split_once('=') {
            match setting.trim() {
                "tabstop" | "ts" => {
                    if let Ok(_width) = value.parse::<usize>() {
                        editor.set_config_setting("tabstop", value);
                        editor.set_status_message(format!("Tab width set to {}", value));
                    } else {
                        editor.set_status_message("Invalid tab width value".to_string());
                    }
                }
                _ => editor.set_status_message(format!("Unknown setting: {}", setting)),
            }
            return;
        }

        // Handle boolean options (enable)
        match args {
            "number" | "nu" => {
                editor.set_config_setting("number", "true");
                let (_, relative) = editor.get_line_number_state();
                editor.set_line_numbers(true, relative);
            }
            "relativenumber" | "rnu" => {
                editor.set_config_setting("relativenumber", "true");
                let (absolute, _) = editor.get_line_number_state();
                editor.set_line_numbers(absolute, true);
            }
            "cursorline" | "cul" => {
                editor.set_config_setting("cursorline", "true");
                editor.set_cursor_line(true);
            }
            _ => editor.set_status_message(format!("Unknown option: {}", args)),
        }
    }
}
