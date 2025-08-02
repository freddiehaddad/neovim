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
    pub visual_line_mode: HashMap<String, String>,
    pub visual_block_mode: HashMap<String, String>,
    pub replace_mode: HashMap<String, String>,
    pub search_mode: HashMap<String, String>,
}

#[derive(Clone)]
pub struct KeyHandler {
    keymap_config: KeymapConfig,
    pending_sequence: String,
    last_key_time: Option<std::time::Instant>,
}

impl Default for KeyHandler {
    fn default() -> Self {
        Self::new()
    }
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
        // Try to load from keymaps.toml file
        if let Ok(config_content) = fs::read_to_string("keymaps.toml") {
            if let Ok(config) = toml::from_str(&config_content) {
                return config;
            }
        }

        // Fallback to empty keymaps - this should rarely happen
        // Users should have a keymaps.toml file
        eprintln!("Warning: Could not load keymaps.toml, using minimal fallback");
        Self::create_minimal_fallback()
    }

    fn create_minimal_fallback() -> KeymapConfig {
        // Absolute minimal keymaps just to exit gracefully
        let mut normal_mode = HashMap::new();
        normal_mode.insert(":".to_string(), "command_mode".to_string());

        let mut command_mode = HashMap::new();
        command_mode.insert("Escape".to_string(), "normal_mode".to_string());
        command_mode.insert("Enter".to_string(), "execute_command".to_string());
        command_mode.insert("Char".to_string(), "append_command".to_string());

        KeymapConfig {
            normal_mode,
            insert_mode: HashMap::new(),
            command_mode,
            visual_mode: HashMap::new(),
            visual_line_mode: HashMap::new(),
            visual_block_mode: HashMap::new(),
            replace_mode: HashMap::new(),
            search_mode: HashMap::new(),
        }
    }

    pub fn handle_key(&mut self, editor: &mut Editor, key: KeyEvent) -> Result<()> {
        let key_string = Self::key_event_to_string(key);

        // Explicit escape-from-command-mode shortcut so suggestions get cleared (fallback)
            if editor.mode() == Mode::Command && key.code == KeyCode::Esc {
                editor.set_mode(Mode::Normal);
                return Ok(());
            }
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
                // For single character keys after single character sequences, concatenate without space
                // Example: "g" + "g" = "gg"
                if key_string.len() == 1
                    && self.pending_sequence.len() == 1
                    && self
                        .pending_sequence
                        .chars()
                        .next()
                        .unwrap_or(' ')
                        .is_ascii_alphabetic()
                {
                    self.pending_sequence.push_str(&key_string);
                } else {
                    // For all other cases, add space between keys
                    // Example: "Ctrl+w" + "h" = "Ctrl+w h"
                    self.pending_sequence.push(' ');
                    self.pending_sequence.push_str(&key_string);
                }
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
            Mode::Visual => self.keymap_config.visual_mode.get(&key_string),
            Mode::VisualLine => self.keymap_config.visual_line_mode.get(&key_string),
            Mode::VisualBlock => self.keymap_config.visual_block_mode.get(&key_string),
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
        let mut result = String::new();

        // Add modifiers
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            result.push_str("Ctrl+");
        }
        if key.modifiers.contains(KeyModifiers::ALT) {
            result.push_str("Alt+");
        }
        if key.modifiers.contains(KeyModifiers::SHIFT) {
            result.push_str("Shift+");
        }

        // Add the key itself
        match key.code {
            KeyCode::Char(c) => {
                // Don't add Shift+ for uppercase letters as they're already shifted
                if key.modifiers.contains(KeyModifiers::SHIFT) && c.is_ascii_lowercase() {
                    result.truncate(result.len() - 6); // Remove "Shift+"
                    result.push(c.to_ascii_uppercase());
                } else if key.modifiers.contains(KeyModifiers::SHIFT) && c.is_ascii_uppercase() {
                    result.truncate(result.len() - 6); // Remove "Shift+"
                    result.push(c);
                } else if key.modifiers.contains(KeyModifiers::SHIFT)
                    && "!@#$%^&*()_+{}|:\"<>?~".contains(c)
                {
                    // For shifted special characters, remove Shift+ as the character itself represents the shifted version
                    result.truncate(result.len() - 6); // Remove "Shift+"
                    result.push(c);
                } else {
                    result.push(c);
                }
            }
            KeyCode::Enter => result.push_str("Enter"),
            KeyCode::Left => result.push_str("Left"),
            KeyCode::Right => result.push_str("Right"),
            KeyCode::Up => result.push_str("Up"),
            KeyCode::Down => result.push_str("Down"),
            KeyCode::Backspace => result.push_str("Backspace"),
            KeyCode::Esc => result.push_str("Escape"),
            KeyCode::Tab => result.push_str("Tab"),
            KeyCode::Delete => result.push_str("Delete"),
            KeyCode::Home => result.push_str("Home"),
            KeyCode::End => result.push_str("End"),
            KeyCode::PageUp => result.push_str("PageUp"),
            KeyCode::PageDown => result.push_str("PageDown"),
            KeyCode::Insert => result.push_str("Insert"),
            KeyCode::F(n) => result.push_str(&format!("F{}", n)),
            _ => result.push_str("Unknown"),
        }

        result
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

            // Buffer management actions
            "buffer_next" => self.action_buffer_next(editor)?,
            "buffer_previous" => self.action_buffer_previous(editor)?,

            // Insert mode actions
            "insert_char" => self.action_insert_char(editor, key)?,
            "new_line" => self.action_new_line(editor)?,
            "delete_char" => self.action_delete_char(editor)?,
            "delete_char_forward" => self.action_delete_char_forward(editor)?,
            "delete_word_backward" => self.action_delete_word_backward(editor)?,
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

            // Scrolling actions
            "scroll_down_line" => self.action_scroll_down_line(editor)?,
            "scroll_up_line" => self.action_scroll_up_line(editor)?,
            "scroll_down_page" => self.action_scroll_down_page(editor)?,
            "scroll_up_page" => self.action_scroll_up_page(editor)?,
            "scroll_down_half_page" => self.action_scroll_down_half_page(editor)?,
            "scroll_up_half_page" => self.action_scroll_up_half_page(editor)?,

            // Centering actions (z commands)
            "center_cursor" => self.action_center_cursor(editor)?,
            "cursor_to_top" => self.action_cursor_to_top(editor)?,
            "cursor_to_bottom" => self.action_cursor_to_bottom(editor)?,

            // Window/Split actions
            "split_horizontal" => self.action_split_horizontal(editor)?,
            "split_vertical" => self.action_split_vertical(editor)?,
            "split_horizontal_above" => self.action_split_horizontal_above(editor)?,
            "split_horizontal_below" => self.action_split_horizontal_below(editor)?,
            "split_vertical_left" => self.action_split_vertical_left(editor)?,
            "split_vertical_right" => self.action_split_vertical_right(editor)?,
            "close_window" => self.action_close_window(editor)?,
            "window_left" => self.action_window_left(editor)?,
            "window_right" => self.action_window_right(editor)?,
            "window_up" => self.action_window_up(editor)?,
            "window_down" => self.action_window_down(editor)?,


            // Tab Auto Complete
            "cycle_suggestion" => self.action_cycle_suggestion(editor)?,
            "accept_suggestion" => self.action_accept_suggestion(editor)?,

            // Window resizing actions
            "resize_window_wider" => self.action_resize_window_wider(editor)?,
            "resize_window_narrower" => self.action_resize_window_narrower(editor)?,
            "resize_window_taller" => self.action_resize_window_taller(editor)?,
            "resize_window_shorter" => self.action_resize_window_shorter(editor)?,


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

    fn action_cycle_suggestion(&self, editor: &mut Editor) -> Result<()> {
        editor.cycle_suggestion();
        Ok(())
    }
    fn action_accept_suggestion(&self, editor: &mut Editor) -> Result<()> {
        editor.accept_current_suggestion();
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

            // Buffer management commands
            "bn" | "bnext" => {
                if editor.switch_to_next_buffer() {
                    editor.set_status_message("Switched to next buffer".to_string());
                } else {
                    editor.set_status_message("No next buffer".to_string());
                }
            }
            "bp" | "bprev" | "bprevious" => {
                if editor.switch_to_previous_buffer() {
                    editor.set_status_message("Switched to previous buffer".to_string());
                } else {
                    editor.set_status_message("No previous buffer".to_string());
                }
            }
            "bd" | "bdelete" => match editor.close_current_buffer() {
                Ok(msg) => editor.set_status_message(msg),
                Err(e) => editor.set_status_message(format!("Error: {}", e)),
            },
            "bd!" | "bdelete!" => match editor.force_close_current_buffer() {
                Ok(msg) => editor.set_status_message(msg),
                Err(e) => editor.set_status_message(format!("Error: {}", e)),
            },
            "ls" | "buffers" => {
                let buffer_list = editor.list_buffers();
                editor.set_status_message(buffer_list);
            }
            // Window/Split commands
            "split" | "sp" => {
                let message = editor.split_horizontal();
                editor.set_status_message(message);
            }
            "vsplit" | "vsp" => {
                let message = editor.split_vertical();
                editor.set_status_message(message);
            }
            "close" => {
                let message = editor.close_window();
                editor.set_status_message(message);
            }
            _ => {
                // Handle :e filename and :b commands
                if command.starts_with("e ") {
                    let filename = command[2..].trim();
                    match editor.open_file(filename) {
                        Ok(msg) => editor.set_status_message(msg),
                        Err(e) => editor.set_status_message(format!("Error opening file: {}", e)),
                    }
                } else if command.starts_with("b ") {
                    let buffer_ref = command[2..].trim();
                    if let Ok(buffer_id) = buffer_ref.parse::<usize>() {
                        if editor.switch_to_buffer(buffer_id) {
                            editor.set_status_message(format!("Switched to buffer {}", buffer_id));
                        } else {
                            editor.set_status_message(format!("No buffer with ID {}", buffer_id));
                        }
                    } else {
                        // Try to switch by filename
                        if editor.switch_to_buffer_by_name(buffer_ref) {
                            editor
                                .set_status_message(format!("Switched to buffer '{}'", buffer_ref));
                        } else {
                            editor
                                .set_status_message(format!("No buffer matching '{}'", buffer_ref));
                        }
                    }
                } else if command.starts_with("set ") {
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

    fn action_buffer_next(&self, editor: &mut Editor) -> Result<()> {
        editor.switch_to_next_buffer();
        Ok(())
    }

    fn action_buffer_previous(&self, editor: &mut Editor) -> Result<()> {
        editor.switch_to_previous_buffer();
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

    fn action_delete_word_backward(&self, editor: &mut Editor) -> Result<()> {
        if let Some(buffer) = editor.current_buffer_mut() {
            if let Some(line) = buffer.lines.get_mut(buffer.cursor.row) {
                if buffer.cursor.col > 0 {
                    // Find the start of the current word or previous word
                    let mut pos = buffer.cursor.col;

                    // Skip any whitespace before the cursor
                    while pos > 0 && line.chars().nth(pos - 1).unwrap_or(' ').is_whitespace() {
                        pos -= 1;
                    }

                    // Delete the word characters
                    while pos > 0 && !line.chars().nth(pos - 1).unwrap_or(' ').is_whitespace() {
                        pos -= 1;
                    }

                    // Remove the characters from pos to cursor
                    line.drain(pos..buffer.cursor.col);
                    buffer.cursor.col = pos;
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

    // Scrolling action implementations
    fn action_scroll_down_line(&self, editor: &mut Editor) -> Result<()> {
        editor.scroll_down_line();
        Ok(())
    }

    fn action_scroll_up_line(&self, editor: &mut Editor) -> Result<()> {
        editor.scroll_up_line();
        Ok(())
    }

    fn action_scroll_down_page(&self, editor: &mut Editor) -> Result<()> {
        editor.scroll_down_page();
        Ok(())
    }

    fn action_scroll_up_page(&self, editor: &mut Editor) -> Result<()> {
        editor.scroll_up_page();
        Ok(())
    }

    fn action_scroll_down_half_page(&self, editor: &mut Editor) -> Result<()> {
        editor.scroll_down_half_page();
        Ok(())
    }

    fn action_scroll_up_half_page(&self, editor: &mut Editor) -> Result<()> {
        editor.scroll_up_half_page();
        Ok(())
    }

    // Centering action implementations (z commands)
    fn action_center_cursor(&self, editor: &mut Editor) -> Result<()> {
        editor.center_cursor();
        Ok(())
    }

    fn action_cursor_to_top(&self, editor: &mut Editor) -> Result<()> {
        editor.cursor_to_top();
        Ok(())
    }

    fn action_cursor_to_bottom(&self, editor: &mut Editor) -> Result<()> {
        editor.cursor_to_bottom();
        Ok(())
    }

    // Window/Split action implementations
    fn action_split_horizontal(&self, editor: &mut Editor) -> Result<()> {
        let message = editor.split_horizontal();
        editor.set_status_message(message);
        Ok(())
    }

    fn action_split_vertical(&self, editor: &mut Editor) -> Result<()> {
        let message = editor.split_vertical();
        editor.set_status_message(message);
        Ok(())
    }

    fn action_split_horizontal_above(&self, editor: &mut Editor) -> Result<()> {
        let message = editor.split_horizontal_above();
        editor.set_status_message(message);
        Ok(())
    }

    fn action_split_horizontal_below(&self, editor: &mut Editor) -> Result<()> {
        let message = editor.split_horizontal_below();
        editor.set_status_message(message);
        Ok(())
    }

    fn action_split_vertical_left(&self, editor: &mut Editor) -> Result<()> {
        let message = editor.split_vertical_left();
        editor.set_status_message(message);
        Ok(())
    }

    fn action_split_vertical_right(&self, editor: &mut Editor) -> Result<()> {
        let message = editor.split_vertical_right();
        editor.set_status_message(message);
        Ok(())
    }

    fn action_close_window(&self, editor: &mut Editor) -> Result<()> {
        let message = editor.close_window();
        editor.set_status_message(message);
        Ok(())
    }

    fn action_window_left(&self, editor: &mut Editor) -> Result<()> {
        if !editor.move_to_window_left() {
            editor.set_status_message("No window to the left".to_string());
        }
        Ok(())
    }

    fn action_window_right(&self, editor: &mut Editor) -> Result<()> {
        if !editor.move_to_window_right() {
            editor.set_status_message("No window to the right".to_string());
        }
        Ok(())
    }

    fn action_window_up(&self, editor: &mut Editor) -> Result<()> {
        if !editor.move_to_window_up() {
            editor.set_status_message("No window above".to_string());
        }
        Ok(())
    }

    fn action_window_down(&self, editor: &mut Editor) -> Result<()> {
        if !editor.move_to_window_down() {
            editor.set_status_message("No window below".to_string());
        }
        Ok(())
    }

    // Window resizing action implementations
    fn action_resize_window_wider(&self, editor: &mut Editor) -> Result<()> {
        let message = editor.resize_window_wider();
        editor.set_status_message(message);
        Ok(())
    }

    fn action_resize_window_narrower(&self, editor: &mut Editor) -> Result<()> {
        let message = editor.resize_window_narrower();
        editor.set_status_message(message);
        Ok(())
    }

    fn action_resize_window_taller(&self, editor: &mut Editor) -> Result<()> {
        let message = editor.resize_window_taller();
        editor.set_status_message(message);
        Ok(())
    }

    fn action_resize_window_shorter(&self, editor: &mut Editor) -> Result<()> {
        let message = editor.resize_window_shorter();
        editor.set_status_message(message);
        Ok(())
    }

    /// Handle generic :set commands
    fn handle_set_command(&self, editor: &mut Editor, args: &str) {
        let args = args.trim();

        // Handle empty set command - show some basic settings
        if args.is_empty() {
            let mut settings = Vec::new();
            settings.push(format!(
                "number: {}",
                editor.get_config_value("number").unwrap_or_default()
            ));
            settings.push(format!(
                "relativenumber: {}",
                editor
                    .get_config_value("relativenumber")
                    .unwrap_or_default()
            ));
            settings.push(format!(
                "cursorline: {}",
                editor.get_config_value("cursorline").unwrap_or_default()
            ));
            settings.push(format!(
                "tabstop: {}",
                editor.get_config_value("tabstop").unwrap_or_default()
            ));
            settings.push(format!(
                "expandtab: {}",
                editor.get_config_value("expandtab").unwrap_or_default()
            ));
            editor.set_status_message(format!("Current settings: {}", settings.join(", ")));
            return;
        }

        // Handle :set all command
        if args == "all" {
            let all_settings = [
                "number",
                "relativenumber",
                "cursorline",
                "tabstop",
                "expandtab",
                "autoindent",
                "ignorecase",
                "smartcase",
                "hlsearch",
                "incsearch",
                "wrap",
                "linebreak",
                "undolevels",
                "undofile",
                "backup",
                "swapfile",
                "autosave",
                "laststatus",
                "showcmd",
                "scrolloff",
                "sidescrolloff",
                "timeoutlen",
                "colorscheme",
                "syntax",
            ];

            let mut all_values = Vec::new();
            for setting in &all_settings {
                if let Some(value) = editor.get_config_value(setting) {
                    all_values.push(format!("{}={}", setting, value));
                }
            }

            editor.set_status_message(format!("All settings: {}", all_values.join(" | ")));
            return;
        }

        // Handle query for specific setting (e.g., "set number?")
        if let Some(setting) = args.strip_suffix('?') {
            if let Some(value) = editor.get_config_value(setting) {
                editor.set_status_message(format!("{}: {}", setting, value));
            } else {
                editor.set_status_message(format!("Unknown setting: {}", setting));
            }
            return;
        }

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
                "ignorecase" | "ic" => {
                    editor.set_config_setting("ignorecase", "false");
                }
                "smartcase" | "scs" => {
                    editor.set_config_setting("smartcase", "false");
                }
                "hlsearch" | "hls" => {
                    editor.set_config_setting("hlsearch", "false");
                }
                "expandtab" | "et" => {
                    editor.set_config_setting("expandtab", "false");
                }
                "autoindent" | "ai" => {
                    editor.set_config_setting("autoindent", "false");
                }
                "incsearch" | "is" => {
                    editor.set_config_setting("incsearch", "false");
                }
                "wrap" => {
                    editor.set_config_setting("wrap", "false");
                }
                "linebreak" | "lbr" => {
                    editor.set_config_setting("linebreak", "false");
                }
                "undofile" | "udf" => {
                    editor.set_config_setting("undofile", "false");
                }
                "backup" | "bk" => {
                    editor.set_config_setting("backup", "false");
                }
                "swapfile" | "swf" => {
                    editor.set_config_setting("swapfile", "false");
                }
                "autosave" | "aw" => {
                    editor.set_config_setting("autosave", "false");
                }
                "laststatus" | "ls" => {
                    editor.set_config_setting("laststatus", "false");
                }
                "showcmd" | "sc" => {
                    editor.set_config_setting("showcmd", "false");
                }
                "syntax" | "syn" => {
                    editor.set_config_setting("syntax", "false");
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
                "undolevels" | "ul" => {
                    if let Ok(_levels) = value.parse::<usize>() {
                        editor.set_config_setting("undolevels", value);
                        editor.set_status_message(format!("Undo levels set to {}", value));
                    } else {
                        editor.set_status_message("Invalid undo levels value".to_string());
                    }
                }
                "scrolloff" | "so" => {
                    if let Ok(_lines) = value.parse::<usize>() {
                        editor.set_config_setting("scrolloff", value);
                        editor.set_status_message(format!("Scroll offset set to {}", value));
                    } else {
                        editor.set_status_message("Invalid scroll offset value".to_string());
                    }
                }
                "sidescrolloff" | "siso" => {
                    if let Ok(_cols) = value.parse::<usize>() {
                        editor.set_config_setting("sidescrolloff", value);
                        editor.set_status_message(format!("Side scroll offset set to {}", value));
                    } else {
                        editor.set_status_message("Invalid side scroll offset value".to_string());
                    }
                }
                "timeoutlen" | "tm" => {
                    if let Ok(_timeout) = value.parse::<u64>() {
                        editor.set_config_setting("timeoutlen", value);
                        editor.set_status_message(format!("Command timeout set to {} ms", value));
                    } else {
                        editor.set_status_message("Invalid timeout value".to_string());
                    }
                }
                "colorscheme" | "colo" => {
                    editor.set_config_setting("colorscheme", value);
                    editor.set_status_message(format!("Color scheme set to {}", value));
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
            "ignorecase" | "ic" => {
                editor.set_config_setting("ignorecase", "true");
                editor.set_status_message("Ignore case enabled".to_string());
            }
            "smartcase" | "scs" => {
                editor.set_config_setting("smartcase", "true");
                editor.set_status_message("Smart case enabled".to_string());
            }
            "hlsearch" | "hls" => {
                editor.set_config_setting("hlsearch", "true");
                editor.set_status_message("Search highlighting enabled".to_string());
            }
            "expandtab" | "et" => {
                editor.set_config_setting("expandtab", "true");
                editor.set_status_message("Expand tabs enabled".to_string());
            }
            "autoindent" | "ai" => {
                editor.set_config_setting("autoindent", "true");
                editor.set_status_message("Auto indent enabled".to_string());
            }
            "incsearch" | "is" => {
                editor.set_config_setting("incsearch", "true");
                editor.set_status_message("Incremental search enabled".to_string());
            }
            "wrap" => {
                editor.set_config_setting("wrap", "true");
                editor.set_status_message("Line wrap enabled".to_string());
            }
            "linebreak" | "lbr" => {
                editor.set_config_setting("linebreak", "true");
                editor.set_status_message("Line break enabled".to_string());
            }
            "undofile" | "udf" => {
                editor.set_config_setting("undofile", "true");
                editor.set_status_message("Persistent undo enabled".to_string());
            }
            "backup" | "bk" => {
                editor.set_config_setting("backup", "true");
                editor.set_status_message("Backup files enabled".to_string());
            }
            "swapfile" | "swf" => {
                editor.set_config_setting("swapfile", "true");
                editor.set_status_message("Swap file enabled".to_string());
            }
            "autosave" | "aw" => {
                editor.set_config_setting("autosave", "true");
                editor.set_status_message("Auto save enabled".to_string());
            }
            "laststatus" | "ls" => {
                editor.set_config_setting("laststatus", "true");
                editor.set_status_message("Status line enabled".to_string());
            }
            "showcmd" | "sc" => {
                editor.set_config_setting("showcmd", "true");
                editor.set_status_message("Show command enabled".to_string());
            }
            "syntax" | "syn" => {
                editor.set_config_setting("syntax", "true");
                editor.set_status_message("Syntax highlighting enabled".to_string());
            }
            _ => editor.set_status_message(format!("Unknown option: {}", args)),
        }
    }
}
