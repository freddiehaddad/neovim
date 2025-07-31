// Configuration management
// This will handle vimrc/init.vim parsing and settings

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

impl Default for Config {
    fn default() -> Self {
        let mut settings = HashMap::new();

        // Default Vim settings
        settings.insert("number".to_string(), ConfigValue::Bool(false));
        settings.insert("relativenumber".to_string(), ConfigValue::Bool(false));
        settings.insert("tabstop".to_string(), ConfigValue::Int(4));
        settings.insert("shiftwidth".to_string(), ConfigValue::Int(4));
        settings.insert("expandtab".to_string(), ConfigValue::Bool(true));
        settings.insert("autoindent".to_string(), ConfigValue::Bool(true));
        settings.insert("hlsearch".to_string(), ConfigValue::Bool(true));
        settings.insert("incsearch".to_string(), ConfigValue::Bool(true));

        Self {
            settings,
            keymaps: HashMap::new(),
            plugins: Vec::new(),
        }
    }
}

// TODO: Implement config file parsing and loading
