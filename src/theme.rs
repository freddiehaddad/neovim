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
    pub syntax: SyntaxColors,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UIColors {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntaxColors {
    pub text: String,
    pub comment: String,
    pub keyword: String,
    pub operator: String,
    pub r#type: String,
    pub r#struct: String,
    pub r#enum: String,
    pub string: String,
    pub number: String,
    pub boolean: String,
    pub character: String,
    pub function: String,
    pub method: String,
    pub r#macro: String,
    pub variable: String,
    pub parameter: String,
    pub property: String,
    pub constant: String,
}

/// UI theme that uses colors from themes.toml
#[derive(Debug, Clone)]
pub struct UITheme {
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

/// Syntax theme that uses colors from themes.toml
#[derive(Debug, Clone)]
pub struct SyntaxTheme {
    pub text: Color,
    pub comment: Color,
    pub keyword: Color,
    pub operator: Color,
    pub type_color: Color,
    pub struct_color: Color,
    pub enum_color: Color,
    pub string: Color,
    pub number: Color,
    pub boolean: Color,
    pub character: Color,
    pub function: Color,
    pub method: Color,
    pub macro_color: Color,
    pub variable: Color,
    pub parameter: Color,
    pub property: Color,
    pub constant: Color,
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
    pub fn load() -> Self {
        if let Ok(config_content) = fs::read_to_string("themes.toml") {
            if let Ok(config) = toml::from_str(&config_content) {
                return config;
            }
        }

        // Fallback to default configuration if loading fails
        Self::default()
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
                syntax: SyntaxTheme::from_colors(&theme.syntax),
            }
        } else {
            // Fallback to default theme
            self.get_theme("default")
                .unwrap_or_else(|| self.create_fallback_theme())
        }
    }

    /// Get a specific theme by name
    pub fn get_theme(&self, theme_name: &str) -> Option<CompleteTheme> {
        self.themes.get(theme_name).map(|theme| CompleteTheme {
            name: theme.name.clone(),
            description: theme.description.clone(),
            ui: UITheme::from_colors(&theme.ui),
            syntax: SyntaxTheme::from_colors(&theme.syntax),
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

    /// Create a fallback theme if themes.toml is corrupted
    fn create_fallback_theme(&self) -> CompleteTheme {
        CompleteTheme {
            name: "Fallback".to_string(),
            description: "Emergency fallback theme".to_string(),
            ui: UITheme::fallback(),
            syntax: SyntaxTheme::fallback(),
        }
    }
}

impl UITheme {
    /// Create UITheme from color strings in themes.toml
    pub fn from_colors(colors: &UIColors) -> Self {
        Self {
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

    /// Fallback UI theme with basic colors
    pub fn fallback() -> Self {
        Self {
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
    /// Create SyntaxTheme from color strings in themes.toml
    pub fn from_colors(colors: &SyntaxColors) -> Self {
        Self {
            text: parse_color(&colors.text),
            comment: parse_color(&colors.comment),
            keyword: parse_color(&colors.keyword),
            operator: parse_color(&colors.operator),
            type_color: parse_color(&colors.r#type),
            struct_color: parse_color(&colors.r#struct),
            enum_color: parse_color(&colors.r#enum),
            string: parse_color(&colors.string),
            number: parse_color(&colors.number),
            boolean: parse_color(&colors.boolean),
            character: parse_color(&colors.character),
            function: parse_color(&colors.function),
            method: parse_color(&colors.method),
            macro_color: parse_color(&colors.r#macro),
            variable: parse_color(&colors.variable),
            parameter: parse_color(&colors.parameter),
            property: parse_color(&colors.property),
            constant: parse_color(&colors.constant),
        }
    }

    /// Fallback syntax theme with basic colors
    pub fn fallback() -> Self {
        Self {
            text: Color::White,
            comment: Color::DarkGrey,
            keyword: Color::Blue,
            operator: Color::Cyan,
            type_color: Color::Green,
            struct_color: Color::Green,
            enum_color: Color::Green,
            string: Color::Yellow,
            number: Color::Magenta,
            boolean: Color::Cyan,
            character: Color::Yellow,
            function: Color::Blue,
            method: Color::Blue,
            macro_color: Color::Magenta,
            variable: Color::White,
            parameter: Color::Red,
            property: Color::Cyan,
            constant: Color::Magenta,
        }
    }
}

impl Default for ThemeConfig {
    fn default() -> Self {
        // Create basic fallback configuration
        let mut themes = HashMap::new();

        // Add a minimal default theme
        themes.insert(
            "default".to_string(),
            Theme {
                name: "Default".to_string(),
                description: "Basic default theme".to_string(),
                ui: UIColors {
                    status_bg: "#86b300".to_string(),
                    status_fg: "#dbd7ca".to_string(),
                    status_modified: "#ce422b".to_string(),
                    line_number: "#5c6773".to_string(),
                    line_number_current: "#f79718".to_string(),
                    cursor_line_bg: "#2d2d2d".to_string(),
                    empty_line: "#39adb5".to_string(),
                    command_line_bg: "#1c1c1c".to_string(),
                    command_line_fg: "#dbd7ca".to_string(),
                    selection_bg: "#59c2ff".to_string(),
                    warning: "#ff9940".to_string(),
                    error: "#ff3333".to_string(),
                },
                syntax: SyntaxColors {
                    text: "#dbd7ca".to_string(),
                    comment: "#5c6773".to_string(),
                    keyword: "#ff6b35".to_string(),
                    operator: "#ff9940".to_string(),
                    r#type: "#86b300".to_string(),
                    r#struct: "#86b300".to_string(),
                    r#enum: "#86b300".to_string(),
                    string: "#a8cc8c".to_string(),
                    number: "#d19a66".to_string(),
                    boolean: "#56b6c2".to_string(),
                    character: "#a8cc8c".to_string(),
                    function: "#39adb5".to_string(),
                    method: "#39adb5".to_string(),
                    r#macro: "#c678dd".to_string(),
                    variable: "#dbd7ca".to_string(),
                    parameter: "#e06c75".to_string(),
                    property: "#59c2ff".to_string(),
                    constant: "#f79718".to_string(),
                },
            },
        );

        Self {
            theme: ThemeSelection {
                current: "default".to_string(),
            },
            themes,
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

    // Fallback to white if parsing fails
    Color::White
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_color() {
        assert!(matches!(
            parse_color("#ff0000"),
            Color::Rgb { r: 255, g: 0, b: 0 }
        ));
        assert!(matches!(
            parse_color("#00ff00"),
            Color::Rgb { r: 0, g: 255, b: 0 }
        ));
        assert!(matches!(
            parse_color("#0000ff"),
            Color::Rgb { r: 0, g: 0, b: 255 }
        ));
        assert!(matches!(parse_color("invalid"), Color::White));
    }

    #[test]
    fn test_theme_config_load() {
        let config = ThemeConfig::default();
        assert_eq!(config.theme.current, "default");
        assert!(config.themes.contains_key("default"));
    }
}
