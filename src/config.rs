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
