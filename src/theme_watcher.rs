use crate::theme::ThemeConfig;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

/// Watches themes.toml for changes and provides notifications
pub struct ThemeWatcher {
    _watcher: RecommendedWatcher,
    receiver: Receiver<notify::Result<Event>>,
}

impl ThemeWatcher {
    /// Create a new theme watcher for themes.toml
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let (sender, receiver): (
            Sender<notify::Result<Event>>,
            Receiver<notify::Result<Event>>,
        ) = mpsc::channel();

        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = sender.send(res);
        })?;

        // Watch the themes.toml file
        watcher.watch(Path::new("themes.toml"), RecursiveMode::NonRecursive)?;

        Ok(Self {
            _watcher: watcher,
            receiver,
        })
    }

    /// Check for theme file changes (non-blocking)
    pub fn check_for_changes(&self) -> Result<bool, Box<dyn std::error::Error>> {
        // Use try_recv to avoid blocking
        match self.receiver.try_recv() {
            Ok(Ok(event)) => {
                // Check if this is a file modification event
                match event.kind {
                    EventKind::Modify(_) => {
                        // Check if the modified file is themes.toml
                        if event.paths.iter().any(|path| {
                            path.file_name()
                                .and_then(|name| name.to_str())
                                .map(|name| name == "themes.toml")
                                .unwrap_or(false)
                        }) {
                            return Ok(true);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Err(_)) => {
                // Error in file watching, but continue
            }
            Err(mpsc::TryRecvError::Empty) => {
                // No new events, which is normal
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                // Watcher disconnected
                return Err("Theme watcher disconnected".into());
            }
        }

        Ok(false)
    }

    /// Wait for theme file changes (blocking with timeout)
    pub fn wait_for_changes(&self, timeout: Duration) -> Result<bool, Box<dyn std::error::Error>> {
        match self.receiver.recv_timeout(timeout) {
            Ok(Ok(event)) => match event.kind {
                EventKind::Modify(_) => {
                    if event.paths.iter().any(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .map(|name| name == "themes.toml")
                            .unwrap_or(false)
                    }) {
                        return Ok(true);
                    }
                }
                _ => {}
            },
            Ok(Err(_)) => {
                // Error in file watching
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // No changes within timeout, which is normal
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Theme watcher disconnected".into());
            }
        }

        Ok(false)
    }
}

/// Theme manager that handles loading and hot reloading of themes
pub struct ThemeManager {
    config: ThemeConfig,
    watcher: Option<ThemeWatcher>,
}

impl ThemeManager {
    /// Create a new theme manager with optional file watching
    pub fn new() -> Self {
        let config = ThemeConfig::load();
        let watcher = ThemeWatcher::new().ok(); // Don't fail if watcher can't be created

        Self { config, watcher }
    }

    /// Get the current theme configuration
    pub fn config(&self) -> &ThemeConfig {
        &self.config
    }

    /// Reload themes from themes.toml
    pub fn reload(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        let new_config = ThemeConfig::load();
        let changed = self.config.theme.current != new_config.theme.current
            || self.config.themes.len() != new_config.themes.len();

        self.config = new_config;
        Ok(changed)
    }

    /// Check for theme file changes and reload if necessary
    pub fn check_and_reload(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        if let Some(watcher) = &self.watcher {
            if watcher.check_for_changes()? {
                return self.reload();
            }
        }
        Ok(false)
    }

    /// Set the current theme and save to file
    pub fn set_current_theme(
        &mut self,
        theme_name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.config.themes.contains_key(theme_name) {
            self.config.set_current_theme(theme_name);
            self.config.save()?;
        }
        Ok(())
    }

    /// Get list of available theme names
    pub fn available_themes(&self) -> Vec<&String> {
        self.config.list_themes()
    }

    /// Get the current active theme name
    pub fn current_theme_name(&self) -> &str {
        &self.config.theme.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_manager_creation() {
        let manager = ThemeManager::new();
        assert!(!manager.available_themes().is_empty());
        assert!(!manager.current_theme_name().is_empty());
    }

    #[test]
    fn test_theme_config_reload() {
        let mut manager = ThemeManager::new();

        // Reload should work even if file hasn't changed
        let result = manager.reload();
        assert!(result.is_ok());
    }
}
