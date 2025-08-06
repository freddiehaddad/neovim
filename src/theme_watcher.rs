use crate::theme::ThemeConfig;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
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
            match &res {
                Ok(event) => log::trace!("Theme watcher received event: {:?}", event),
                Err(e) => log::warn!("Theme watcher received error: {:?}", e),
            }
            let _ = sender.send(res);
        })?;

        // Watch the themes.toml file - use absolute path
        let themes_path = std::env::current_dir()?.join("themes.toml");
        log::info!("Watching themes file at: {:?}", themes_path);
        watcher.watch(&themes_path, RecursiveMode::NonRecursive)?;

        Ok(Self {
            _watcher: watcher,
            receiver,
        })
    }

    /// Check for theme file changes (non-blocking)
    pub fn check_for_changes(&self) -> Result<bool, Box<dyn std::error::Error>> {
        let mut theme_changed = false;
        let mut event_count = 0;

        log::trace!("Starting to drain events from theme watcher channel...");

        // Drain all available events to handle multiple rapid file events
        loop {
            match self.receiver.try_recv() {
                Ok(Ok(event)) => {
                    event_count += 1;
                    log::trace!("Processing file event #{}: {:?}", event_count, event);

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
                                log::debug!(
                                    "Detected themes.toml modification in event #{}",
                                    event_count
                                );
                                theme_changed = true;
                            }
                        }
                        _ => {
                            log::trace!("Non-modify event: {:?}", event.kind);
                        }
                    }
                }
                Ok(Err(e)) => {
                    log::warn!("Error in file watching: {:?}", e);
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // No more events available
                    log::trace!(
                        "No more events in channel (processed {} events)",
                        event_count
                    );
                    break;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Watcher disconnected
                    return Err("Theme watcher disconnected".into());
                }
            }
        }

        if theme_changed {
            log::debug!(
                "Theme file change detected after processing {} events",
                event_count
            );
        } else if event_count > 0 {
            log::trace!(
                "Processed {} events but no theme changes detected",
                event_count
            );
        }

        Ok(theme_changed)
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
        let watcher = match ThemeWatcher::new() {
            Ok(w) => {
                log::info!("Theme watcher created successfully");
                Some(w)
            }
            Err(e) => {
                log::warn!("Failed to create theme watcher: {}", e);
                None
            }
        };

        Self { config, watcher }
    }

    /// Get the current theme configuration
    pub fn config(&self) -> &ThemeConfig {
        &self.config
    }

    /// Reload themes from themes.toml
    pub fn reload(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        let new_config = ThemeConfig::load();

        // Check if anything has changed - theme name, theme count, or theme content
        let theme_changed = self.config.theme.current != new_config.theme.current;
        let theme_count_changed = self.config.themes.len() != new_config.themes.len();

        // Check if the content of any theme has changed by comparing the serialized data
        let content_changed = {
            // Convert both configs to strings and compare
            if let (Ok(old_toml), Ok(new_toml)) =
                (toml::to_string(&self.config), toml::to_string(&new_config))
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

        self.config = new_config;
        Ok(any_change)
    }

    /// Check for theme file changes and reload if necessary
    pub fn check_and_reload(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        if let Some(watcher) = &self.watcher {
            log::trace!("Checking for theme file changes...");
            if watcher.check_for_changes()? {
                log::debug!("Theme changes detected, reloading...");
                return self.reload();
            }
        } else {
            log::trace!("No theme watcher available");
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
