use crate::editor::Editor;
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone)]
pub struct KeyHandler;

impl KeyHandler {
    pub fn new() -> Self {
        Self
    }

    pub fn handle_key(&self, editor: &mut Editor, key: KeyEvent) -> Result<()> {
        match editor.mode() {
            Mode::Normal => self.handle_normal_mode(editor, key),
            Mode::Insert => self.handle_insert_mode(editor, key),
            Mode::Command => self.handle_command_mode(editor, key),
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => {
                self.handle_visual_mode(editor, key)
            }
            Mode::Replace => self.handle_replace_mode(editor, key),
            Mode::Search => self.handle_search_mode(editor, key),
        }
    }

    fn handle_normal_mode(&self, editor: &mut Editor, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('i') => {
                editor.set_mode(Mode::Insert);
            }
            KeyCode::Char('I') => {
                // Insert at beginning of line
                if let Some(buffer) = editor.current_buffer_mut() {
                    buffer.cursor.col = 0;
                }
                editor.set_mode(Mode::Insert);
            }
            KeyCode::Char('a') => {
                // Insert after cursor
                if let Some(buffer) = editor.current_buffer_mut() {
                    buffer.cursor.col += 1;
                }
                editor.set_mode(Mode::Insert);
            }
            KeyCode::Char('A') => {
                // Insert at end of line
                if let Some(buffer) = editor.current_buffer_mut() {
                    if let Some(line) = buffer.get_line(buffer.cursor.row) {
                        buffer.cursor.col = line.len();
                    }
                }
                editor.set_mode(Mode::Insert);
            }
            KeyCode::Char('o') => {
                // Open new line below
                if let Some(buffer) = editor.current_buffer_mut() {
                    buffer.cursor.col = 0;
                    buffer.cursor.row += 1;
                    buffer.lines.insert(buffer.cursor.row, String::new());
                    buffer.modified = true;
                }
                editor.set_mode(Mode::Insert);
            }
            KeyCode::Char('O') => {
                // Open new line above
                if let Some(buffer) = editor.current_buffer_mut() {
                    buffer.lines.insert(buffer.cursor.row, String::new());
                    buffer.cursor.col = 0;
                    buffer.modified = true;
                }
                editor.set_mode(Mode::Insert);
            }
            KeyCode::Char('v') => {
                editor.set_mode(Mode::Visual);
            }
            KeyCode::Char('V') => {
                editor.set_mode(Mode::VisualLine);
            }
            KeyCode::Char(':') => {
                editor.set_mode(Mode::Command);
                editor.set_command_line(":".to_string());
            }
            KeyCode::Char('/') => {
                editor.set_mode(Mode::Search);
                editor.set_command_line("/".to_string());
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if let Some(buffer) = editor.current_buffer_mut() {
                    if buffer.cursor.col > 0 {
                        buffer.cursor.col -= 1;
                    }
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(buffer) = editor.current_buffer_mut() {
                    if buffer.cursor.row < buffer.lines.len() - 1 {
                        buffer.cursor.row += 1;
                        // Clamp column to line length
                        if let Some(line) = buffer.get_line(buffer.cursor.row) {
                            buffer.cursor.col = buffer.cursor.col.min(line.len());
                        }
                    }
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(buffer) = editor.current_buffer_mut() {
                    if buffer.cursor.row > 0 {
                        buffer.cursor.row -= 1;
                        // Clamp column to line length
                        if let Some(line) = buffer.get_line(buffer.cursor.row) {
                            buffer.cursor.col = buffer.cursor.col.min(line.len());
                        }
                    }
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if let Some(buffer) = editor.current_buffer_mut() {
                    if let Some(line) = buffer.get_line(buffer.cursor.row) {
                        if buffer.cursor.col < line.len() {
                            buffer.cursor.col += 1;
                        }
                    }
                }
            }
            KeyCode::Char('u') => {
                if let Some(buffer) = editor.current_buffer_mut() {
                    buffer.undo();
                }
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(buffer) = editor.current_buffer_mut() {
                    buffer.redo();
                }
            }
            KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.quit();
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_insert_mode(&self, editor: &mut Editor, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                editor.set_mode(Mode::Normal);
            }
            KeyCode::Char(ch) => {
                if let Some(buffer) = editor.current_buffer_mut() {
                    buffer.insert_char(ch);
                }
            }
            KeyCode::Enter => {
                if let Some(buffer) = editor.current_buffer_mut() {
                    buffer.insert_line_break();
                }
            }
            KeyCode::Backspace => {
                if let Some(buffer) = editor.current_buffer_mut() {
                    buffer.delete_char();
                }
            }
            KeyCode::Left => {
                if let Some(buffer) = editor.current_buffer_mut() {
                    if buffer.cursor.col > 0 {
                        buffer.cursor.col -= 1;
                    }
                }
            }
            KeyCode::Right => {
                if let Some(buffer) = editor.current_buffer_mut() {
                    if let Some(line) = buffer.get_line(buffer.cursor.row) {
                        if buffer.cursor.col < line.len() {
                            buffer.cursor.col += 1;
                        }
                    }
                }
            }
            KeyCode::Up => {
                if let Some(buffer) = editor.current_buffer_mut() {
                    if buffer.cursor.row > 0 {
                        buffer.cursor.row -= 1;
                        if let Some(line) = buffer.get_line(buffer.cursor.row) {
                            buffer.cursor.col = buffer.cursor.col.min(line.len());
                        }
                    }
                }
            }
            KeyCode::Down => {
                if let Some(buffer) = editor.current_buffer_mut() {
                    if buffer.cursor.row < buffer.lines.len() - 1 {
                        buffer.cursor.row += 1;
                        if let Some(line) = buffer.get_line(buffer.cursor.row) {
                            buffer.cursor.col = buffer.cursor.col.min(line.len());
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_command_mode(&self, editor: &mut Editor, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                editor.set_mode(Mode::Normal);
            }
            KeyCode::Enter => {
                let command = editor.command_line().to_string();
                self.execute_command(editor, &command)?;
                editor.set_mode(Mode::Normal);
            }
            KeyCode::Char(ch) => {
                let mut cmd = editor.command_line().to_string();
                cmd.push(ch);
                editor.set_command_line(cmd);
            }
            KeyCode::Backspace => {
                let mut cmd = editor.command_line().to_string();
                if cmd.len() > 1 {  // Keep the ':'
                    cmd.pop();
                    editor.set_command_line(cmd);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_visual_mode(&self, editor: &mut Editor, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                editor.set_mode(Mode::Normal);
            }
            // TODO: Implement visual mode navigation and operations
            _ => {}
        }
        Ok(())
    }

    fn handle_replace_mode(&self, editor: &mut Editor, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                editor.set_mode(Mode::Normal);
            }
            // TODO: Implement replace mode
            _ => {}
        }
        Ok(())
    }

    fn handle_search_mode(&self, editor: &mut Editor, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                editor.set_mode(Mode::Normal);
            }
            KeyCode::Enter => {
                let search_term = &editor.command_line()[1..]; // Remove '/'
                // TODO: Implement search functionality
                editor.set_status_message(format!("Searching for: {}", search_term));
                editor.set_mode(Mode::Normal);
            }
            KeyCode::Char(ch) => {
                let mut search = editor.command_line().to_string();
                search.push(ch);
                editor.set_command_line(search);
            }
            KeyCode::Backspace => {
                let mut search = editor.command_line().to_string();
                if search.len() > 1 {  // Keep the '/'
                    search.pop();
                    editor.set_command_line(search);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn execute_command(&self, editor: &mut Editor, command: &str) -> Result<()> {
        let cmd = command.trim_start_matches(':');
        
        match cmd {
            "q" => {
                editor.quit();
            }
            "q!" => {
                editor.force_quit();
            }
            "w" => {
                editor.save_current_buffer()?;
            }
            "wq" => {
                editor.save_current_buffer()?;
                editor.quit();
            }
            cmd if cmd.starts_with("e ") => {
                let filename = &cmd[2..];
                let path = std::path::PathBuf::from(filename);
                match editor.create_buffer(Some(path)) {
                    Ok(_) => {
                        editor.set_status_message(format!("Opened: {}", filename));
                    }
                    Err(e) => {
                        editor.set_status_message(format!("Error: {}", e));
                    }
                }
            }
            _ => {
                editor.set_status_message(format!("Unknown command: {}", cmd));
            }
        }
        
        Ok(())
    }
}
