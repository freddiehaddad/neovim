// Configuration management
// This handles editor.toml parsing and settings management

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorConfig {
    pub display: DisplayConfig,
    pub behavior: BehaviorConfig,
    pub editing: EditingConfig,
    pub interface: InterfaceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayConfig {
    pub show_line_numbers: bool,
    pub show_relative_numbers: bool,
    pub show_cursor_line: bool,
    pub color_scheme: String,
    pub syntax_highlighting: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorConfig {
    pub tab_width: usize,
    pub expand_tabs: bool,
    pub auto_indent: bool,
    pub ignore_case: bool,
    pub smart_case: bool,
    pub highlight_search: bool,
    pub incremental_search: bool,
    pub wrap_lines: bool,
    pub line_break: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditingConfig {
    pub undo_levels: usize,
    pub persistent_undo: bool,
    pub backup: bool,
    pub swap_file: bool,
    pub auto_save: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceConfig {
    pub show_status_line: bool,
    pub status_line_format: String,
    pub command_timeout: u64,
    pub show_command: bool,
    pub scroll_off: usize,
    pub side_scroll_off: usize,
    pub window_resize_amount: u16,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            display: DisplayConfig {
                show_line_numbers: true,
                show_relative_numbers: false,
                show_cursor_line: false,
                color_scheme: "default".to_string(),
                syntax_highlighting: true,
            },
            behavior: BehaviorConfig {
                tab_width: 4,
                expand_tabs: false,
                auto_indent: true,
                ignore_case: false,
                smart_case: false,
                highlight_search: true,
                incremental_search: true,
                wrap_lines: false,
                line_break: false,
            },
            editing: EditingConfig {
                undo_levels: 1000,
                persistent_undo: false,
                backup: false,
                swap_file: false,
                auto_save: false,
            },
            interface: InterfaceConfig {
                show_status_line: true,
                status_line_format: "default".to_string(),
                command_timeout: 1000,
                show_command: true,
                scroll_off: 0,
                side_scroll_off: 0,
                window_resize_amount: 1,
            },
        }
    }
}

impl EditorConfig {
    pub fn load() -> Self {
        // Try to load from editor.toml file, fall back to defaults if not found
        if let Ok(config_content) = fs::read_to_string("editor.toml") {
            if let Ok(config) = toml::from_str(&config_content) {
                return config;
            }
        }

        // Fallback to default configuration
        Self::default()
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let toml_string = toml::to_string_pretty(self)?;
        fs::write("editor.toml", toml_string)?;
        Ok(())
    }

    /// Update a setting and return success status
    pub fn set_setting(&mut self, setting: &str, value: &str) -> Result<String, String> {
        match setting {
            // Display settings
            "number" | "nu" => {
                self.display.show_line_numbers = value.parse().unwrap_or(true);
                Ok(format!(
                    "Line numbers: {}",
                    if self.display.show_line_numbers {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }
            "relativenumber" | "rnu" => {
                self.display.show_relative_numbers = value.parse().unwrap_or(false);
                Ok(format!(
                    "Relative line numbers: {}",
                    if self.display.show_relative_numbers {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }
            "cursorline" | "cul" => {
                self.display.show_cursor_line = value.parse().unwrap_or(false);
                Ok(format!(
                    "Cursor line: {}",
                    if self.display.show_cursor_line {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }

            // Behavior settings
            "ignorecase" | "ic" => {
                self.behavior.ignore_case = value.parse().unwrap_or(false);
                Ok(format!(
                    "Ignore case: {}",
                    if self.behavior.ignore_case {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }
            "smartcase" | "scs" => {
                self.behavior.smart_case = value.parse().unwrap_or(false);
                Ok(format!(
                    "Smart case: {}",
                    if self.behavior.smart_case {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }
            "hlsearch" | "hls" => {
                self.behavior.highlight_search = value.parse().unwrap_or(true);
                Ok(format!(
                    "Search highlighting: {}",
                    if self.behavior.highlight_search {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }
            "expandtab" | "et" => {
                self.behavior.expand_tabs = value.parse().unwrap_or(false);
                Ok(format!(
                    "Expand tabs: {}",
                    if self.behavior.expand_tabs {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }
            "tabstop" | "ts" => {
                if let Ok(width) = value.parse::<usize>() {
                    self.behavior.tab_width = width;
                    Ok(format!("Tab width: {}", width))
                } else {
                    Err("Invalid tab width".to_string())
                }
            }
            "autoindent" | "ai" => {
                self.behavior.auto_indent = value.parse().unwrap_or(true);
                Ok(format!(
                    "Auto indent: {}",
                    if self.behavior.auto_indent {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }
            "incsearch" | "is" => {
                self.behavior.incremental_search = value.parse().unwrap_or(true);
                Ok(format!(
                    "Incremental search: {}",
                    if self.behavior.incremental_search {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }
            "wrap" => {
                self.behavior.wrap_lines = value.parse().unwrap_or(false);
                Ok(format!(
                    "Line wrap: {}",
                    if self.behavior.wrap_lines {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }
            "linebreak" | "lbr" => {
                self.behavior.line_break = value.parse().unwrap_or(false);
                Ok(format!(
                    "Line break: {}",
                    if self.behavior.line_break {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }

            // Editing settings
            "undolevels" | "ul" => {
                if let Ok(levels) = value.parse::<usize>() {
                    self.editing.undo_levels = levels;
                    Ok(format!("Undo levels: {}", levels))
                } else {
                    Err("Invalid undo levels".to_string())
                }
            }
            "undofile" | "udf" => {
                self.editing.persistent_undo = value.parse().unwrap_or(false);
                Ok(format!(
                    "Persistent undo: {}",
                    if self.editing.persistent_undo {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }
            "backup" | "bk" => {
                self.editing.backup = value.parse().unwrap_or(false);
                Ok(format!(
                    "Backup files: {}",
                    if self.editing.backup {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }
            "swapfile" | "swf" => {
                self.editing.swap_file = value.parse().unwrap_or(false);
                Ok(format!(
                    "Swap file: {}",
                    if self.editing.swap_file {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }
            "autosave" | "aw" => {
                self.editing.auto_save = value.parse().unwrap_or(false);
                Ok(format!(
                    "Auto save: {}",
                    if self.editing.auto_save {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }

            // Interface settings
            "laststatus" | "ls" => {
                self.interface.show_status_line = value.parse().unwrap_or(true);
                Ok(format!(
                    "Status line: {}",
                    if self.interface.show_status_line {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }
            "showcmd" | "sc" => {
                self.interface.show_command = value.parse().unwrap_or(true);
                Ok(format!(
                    "Show command: {}",
                    if self.interface.show_command {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }
            "scrolloff" | "so" => {
                if let Ok(lines) = value.parse::<usize>() {
                    self.interface.scroll_off = lines;
                    Ok(format!("Scroll offset: {}", lines))
                } else {
                    Err("Invalid scroll offset".to_string())
                }
            }
            "sidescrolloff" | "siso" => {
                if let Ok(cols) = value.parse::<usize>() {
                    self.interface.side_scroll_off = cols;
                    Ok(format!("Side scroll offset: {}", cols))
                } else {
                    Err("Invalid side scroll offset".to_string())
                }
            }
            "timeoutlen" | "tm" => {
                if let Ok(timeout) = value.parse::<u64>() {
                    self.interface.command_timeout = timeout;
                    Ok(format!("Command timeout: {} ms", timeout))
                } else {
                    Err("Invalid timeout value".to_string())
                }
            }

            // Display settings
            "colorscheme" | "colo" => {
                self.display.color_scheme = value.to_string();
                Ok(format!("Color scheme: {}", value))
            }
            "syntax" | "syn" => {
                self.display.syntax_highlighting = value.parse().unwrap_or(true);
                Ok(format!(
                    "Syntax highlighting: {}",
                    if self.display.syntax_highlighting {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }

            _ => Err(format!("Unknown setting: {}", setting)),
        }
    }
}

// Legacy config types for backwards compatibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub settings: HashMap<String, ConfigValue>,
    pub keymaps: HashMap<String, String>,
    pub plugins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConfigValue {
    Bool(bool),
    Int(i64),
    String(String),
    List(Vec<String>),
}
