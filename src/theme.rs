use crate::config_watcher::ConfigWatcher;
use crossterm::style::Color;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

/// Complete theme configuration with UI and syntax colors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    pub theme: ThemeSelection,
    pub themes: HashMap<String, Theme>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeSelection {
    pub current: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub description: String,
    pub ui: UIColors,
    pub tree_sitter: HashMap<String, String>, // Direct node type -> color mappings (now required)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UIColors {
    pub background: String,
    pub status_bg: String,
    pub status_fg: String,
    pub status_modified: String,
    pub line_number: String,
    pub line_number_current: String,
    pub cursor_line_bg: String,
    pub empty_line: String,
    pub command_line_bg: String,
    pub command_line_fg: String,
    pub selection_bg: String,
    pub warning: String,
    pub error: String,
}

// Removed SyntaxColors and RustSpecificColors - using only tree_sitter node mappings now

/// UI theme that uses colors from themes.toml
#[derive(Debug, Clone)]
pub struct UITheme {
    pub background: Color,
    pub status_bg: Color,
    pub status_fg: Color,
    pub status_modified: Color,
    pub line_number: Color,
    pub line_number_current: Color,
    pub cursor_line_bg: Color,
    pub empty_line: Color,
    pub command_line_bg: Color,
    pub command_line_fg: Color,
    pub selection_bg: Color,
    pub warning: Color,
    pub error: Color,
}

/// Syntax theme that uses only tree-sitter node type mappings
#[derive(Debug, Clone)]
pub struct SyntaxTheme {
    // Tree-sitter node type mappings - the only source of syntax colors
    pub tree_sitter_mappings: HashMap<String, Color>,
}

/// Combined theme with both UI and syntax colors
#[derive(Debug, Clone)]
pub struct CompleteTheme {
    pub name: String,
    pub description: String,
    pub ui: UITheme,
    pub syntax: SyntaxTheme,
}

impl ThemeConfig {
    /// Load theme configuration from themes.toml
    /// Load theme configuration from themes.toml with fallback to editor.toml default
    pub fn load() -> Self {
        Self::load_with_default_theme("dark") // Default fallback if editor.toml is not available
    }

    /// Load theme configuration with a specific default theme name
    pub fn load_with_default_theme(default_theme: &str) -> Self {
        if let Ok(config_content) = fs::read_to_string("themes.toml") {
            if let Ok(mut config) = toml::from_str::<ThemeConfig>(&config_content) {
                // Ensure the current theme exists, if not use the default from editor.toml
                if !config.themes.contains_key(&config.theme.current) {
                    log::warn!(
                        "Current theme '{}' not found in themes.toml",
                        config.theme.current
                    );

                    // Try to use the default theme from editor.toml
                    if config.themes.contains_key(default_theme) {
                        log::info!("Switching to default theme '{}'", default_theme);
                        config.theme.current = default_theme.to_string();
                    } else if let Some(first_theme_name) = config.themes.keys().next().cloned() {
                        log::warn!(
                            "Default theme '{}' not found, using first available theme '{}'",
                            default_theme,
                            first_theme_name
                        );
                        config.theme.current = first_theme_name;
                    } else {
                        log::error!("No themes found in themes.toml!");
                        return Self::create_emergency_config();
                    }
                }
                return config;
            } else {
                log::error!("Failed to parse themes.toml - invalid TOML format");
            }
        } else {
            log::error!("Failed to read themes.toml file");
        }

        // If we can't load themes.toml, create an emergency minimal config
        log::error!("Creating emergency theme configuration - please check themes.toml");
        Self::create_emergency_config()
    }

    /// Create minimal emergency configuration when themes.toml is missing or invalid
    fn create_emergency_config() -> Self {
        log::warn!("Using emergency theme configuration - please restore themes.toml");
        let mut themes = HashMap::new();

        // Create a single emergency theme using neutral colors
        themes.insert(
            "emergency".to_string(),
            Theme {
                name: "Emergency".to_string(),
                description: "Minimal emergency theme - restore themes.toml".to_string(),
                ui: UIColors {
                    background: "#000000".to_string(),
                    status_bg: "#333333".to_string(),
                    status_fg: "#ffffff".to_string(),
                    status_modified: "#ff0000".to_string(),
                    line_number: "#666666".to_string(),
                    line_number_current: "#ffffff".to_string(),
                    cursor_line_bg: "#222222".to_string(),
                    empty_line: "#444444".to_string(),
                    command_line_bg: "#000000".to_string(),
                    command_line_fg: "#ffffff".to_string(),
                    selection_bg: "#444444".to_string(),
                    warning: "#ffff00".to_string(),
                    error: "#ff0000".to_string(),
                },
                tree_sitter: HashMap::from([
                    ("_default".to_string(), "#ffffff".to_string()),
                    ("line_comment".to_string(), "#666666".to_string()),
                    ("visibility_modifier".to_string(), "#00ffff".to_string()),
                    ("fn".to_string(), "#00ffff".to_string()),
                    ("string_literal".to_string(), "#ffff00".to_string()),
                    ("integer_literal".to_string(), "#ff00ff".to_string()),
                ]),
            },
        );

        Self {
            theme: ThemeSelection {
                current: "emergency".to_string(),
            },
            themes,
        }
    }

    /// Save theme configuration to themes.toml
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let toml_string = toml::to_string_pretty(self)?;
        fs::write("themes.toml", toml_string)?;
        Ok(())
    }

    /// Get the current active theme as a CompleteTheme
    pub fn get_current_theme(&self) -> CompleteTheme {
        let theme_name = &self.theme.current;
        if let Some(theme) = self.themes.get(theme_name) {
            CompleteTheme {
                name: theme.name.clone(),
                description: theme.description.clone(),
                ui: UITheme::from_colors(&theme.ui),
                syntax: SyntaxTheme::from_tree_sitter(&theme.tree_sitter),
            }
        } else {
            // If current theme doesn't exist, use first available theme
            if let Some((first_name, first_theme)) = self.themes.iter().next() {
                log::warn!(
                    "Current theme '{}' not found, using '{}'",
                    theme_name,
                    first_name
                );
                CompleteTheme {
                    name: first_theme.name.clone(),
                    description: first_theme.description.clone(),
                    ui: UITheme::from_colors(&first_theme.ui),
                    syntax: SyntaxTheme::from_tree_sitter(&first_theme.tree_sitter),
                }
            } else {
                // This should never happen as we ensure at least one theme exists
                log::error!("No themes available! This should not happen.");
                self.create_emergency_theme()
            }
        }
    }

    /// Create emergency theme if no themes are available (should rarely happen)
    fn create_emergency_theme(&self) -> CompleteTheme {
        CompleteTheme {
            name: "Emergency".to_string(),
            description: "Emergency fallback theme".to_string(),
            ui: UITheme::emergency(),
            syntax: SyntaxTheme::emergency(),
        }
    }

    /// Get a specific theme by name
    pub fn get_theme(&self, theme_name: &str) -> Option<CompleteTheme> {
        self.themes.get(theme_name).map(|theme| CompleteTheme {
            name: theme.name.clone(),
            description: theme.description.clone(),
            ui: UITheme::from_colors(&theme.ui),
            syntax: SyntaxTheme::from_tree_sitter(&theme.tree_sitter),
        })
    }

    /// Set the current active theme
    pub fn set_current_theme(&mut self, theme_name: &str) {
        if self.themes.contains_key(theme_name) {
            self.theme.current = theme_name.to_string();
        }
    }

    /// List all available theme names
    pub fn list_themes(&self) -> Vec<&String> {
        self.themes.keys().collect()
    }

    /// Get the current active theme name
    pub fn current_theme_name(&self) -> &str {
        &self.theme.current
    }

    /// Reload themes from themes.toml and return true if anything changed
    pub fn reload(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        // Get the current theme name to preserve as default if possible
        let current_theme = self.theme.current.clone();
        let new_config = Self::load_with_default_theme(&current_theme);

        // Check if anything has changed - theme name, theme count, or theme content
        let theme_changed = self.theme.current != new_config.theme.current;
        let theme_count_changed = self.themes.len() != new_config.themes.len();

        // Check if the content of any theme has changed by comparing the serialized data
        let content_changed = {
            // Convert both configs to strings and compare
            if let (Ok(old_toml), Ok(new_toml)) =
                (toml::to_string(self), toml::to_string(&new_config))
            {
                old_toml != new_toml
            } else {
                // If we can't serialize, assume it changed to be safe
                true
            }
        };

        let any_change = theme_changed || theme_count_changed || content_changed;

        if any_change {
            log::info!(
                "Theme configuration changed (theme: {}, count: {}, content: {})",
                theme_changed,
                theme_count_changed,
                content_changed
            );
        }

        *self = new_config;
        Ok(any_change)
    }

    /// Check for theme file changes using the provided watcher and reload if necessary
    pub fn check_and_reload(
        &mut self,
        watcher: &ConfigWatcher,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        log::trace!("Checking for theme file changes...");
        if watcher.check_for_theme_changes() {
            log::debug!("Theme changes detected, reloading...");
            return self.reload();
        }
        Ok(false)
    }
}

impl UITheme {
    /// Create UITheme from color strings in themes.toml
    pub fn from_colors(colors: &UIColors) -> Self {
        Self {
            background: parse_color(&colors.background),
            status_bg: parse_color(&colors.status_bg),
            status_fg: parse_color(&colors.status_fg),
            status_modified: parse_color(&colors.status_modified),
            line_number: parse_color(&colors.line_number),
            line_number_current: parse_color(&colors.line_number_current),
            cursor_line_bg: parse_color(&colors.cursor_line_bg),
            empty_line: parse_color(&colors.empty_line),
            command_line_bg: parse_color(&colors.command_line_bg),
            command_line_fg: parse_color(&colors.command_line_fg),
            selection_bg: parse_color(&colors.selection_bg),
            warning: parse_color(&colors.warning),
            error: parse_color(&colors.error),
        }
    }

    /// Emergency UI theme with basic terminal colors (no hard-coded hex values)
    pub fn emergency() -> Self {
        Self {
            background: Color::Black,
            status_bg: Color::DarkGreen,
            status_fg: Color::White,
            status_modified: Color::Red,
            line_number: Color::DarkGrey,
            line_number_current: Color::Yellow,
            cursor_line_bg: Color::DarkGrey,
            empty_line: Color::Blue,
            command_line_bg: Color::Black,
            command_line_fg: Color::White,
            selection_bg: Color::Blue,
            warning: Color::Yellow,
            error: Color::Red,
        }
    }
}

impl SyntaxTheme {
    /// Create SyntaxTheme from tree-sitter mappings in themes.toml
    pub fn from_tree_sitter(tree_sitter: &HashMap<String, String>) -> Self {
        // Build tree-sitter mappings
        let mut tree_sitter_mappings = HashMap::new();
        for (node_type, color_str) in tree_sitter {
            tree_sitter_mappings.insert(node_type.clone(), parse_color(color_str));
        }

        Self {
            tree_sitter_mappings,
        }
    }

    /// Get default text color from tree-sitter mappings
    pub fn get_default_text_color(&self) -> crossterm::style::Color {
        self.tree_sitter_mappings
            .get("_default")
            .cloned()
            .unwrap_or(crossterm::style::Color::White)
    }

    /// Emergency syntax theme with basic terminal colors (no hard-coded hex values)
    pub fn emergency() -> Self {
        let mut tree_sitter_mappings = HashMap::new();

        // Basic emergency colors
        tree_sitter_mappings.insert("_default".to_string(), Color::White);
        tree_sitter_mappings.insert("line_comment".to_string(), Color::DarkGrey);
        tree_sitter_mappings.insert("visibility_modifier".to_string(), Color::Blue);
        tree_sitter_mappings.insert("fn".to_string(), Color::Blue);
        tree_sitter_mappings.insert("string_literal".to_string(), Color::Yellow);
        tree_sitter_mappings.insert("integer_literal".to_string(), Color::Magenta);
        tree_sitter_mappings.insert("type_identifier".to_string(), Color::Green);
        tree_sitter_mappings.insert("identifier".to_string(), Color::White);

        Self {
            tree_sitter_mappings,
        }
    }
}

/// Parse a hex color string to crossterm Color
fn parse_color(color_str: &str) -> Color {
    if let Some(stripped) = color_str.strip_prefix('#') {
        if stripped.len() == 6 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&stripped[0..2], 16),
                u8::from_str_radix(&stripped[2..4], 16),
                u8::from_str_radix(&stripped[4..6], 16),
            ) {
                return Color::Rgb { r, g, b };
            }
        }
    }

    // Fallback to white if parsing fails - this should rarely happen
    // since we now ensure themes.toml always exists
    log::warn!(
        "Failed to parse color '{}', using white fallback",
        color_str
    );
    Color::White
}

#[cfg(test)]
mod tests;
