use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum ConfigChangeEvent {
    EditorConfigChanged,
    KeymapConfigChanged,
}

pub struct ConfigWatcher {
    _watcher: RecommendedWatcher,
    receiver: Receiver<ConfigChangeEvent>,
}

impl ConfigWatcher {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let (tx, rx) = mpsc::channel();

        let watcher = Self::create_watcher(tx)?;

        Ok(ConfigWatcher {
            _watcher: watcher,
            receiver: rx,
        })
    }

    fn create_watcher(
        tx: Sender<ConfigChangeEvent>,
    ) -> Result<RecommendedWatcher, Box<dyn std::error::Error>> {
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| match res {
                Ok(event) => {
                    if let EventKind::Modify(_) = event.kind {
                        for path in event.paths {
                            if let Some(file_name) = path.file_name() {
                                match file_name.to_string_lossy().as_ref() {
                                    "editor.toml" => {
                                        let _ = tx.send(ConfigChangeEvent::EditorConfigChanged);
                                    }
                                    "keymaps.toml" => {
                                        let _ = tx.send(ConfigChangeEvent::KeymapConfigChanged);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Config watcher error: {:?}", e);
                }
            },
            Config::default(),
        )?;

        // Watch the current directory for config file changes
        watcher.watch(Path::new("."), RecursiveMode::NonRecursive)?;

        Ok(watcher)
    }

    /// Check for configuration changes (non-blocking)
    pub fn check_for_changes(&self) -> Vec<ConfigChangeEvent> {
        let mut changes = Vec::new();

        // Collect all pending events
        while let Ok(event) = self.receiver.try_recv() {
            changes.push(event);
        }

        changes
    }

    /// Wait for a configuration change with timeout
    pub fn wait_for_change(&self, timeout: Duration) -> Option<ConfigChangeEvent> {
        match self.receiver.recv_timeout(timeout) {
            Ok(event) => Some(event),
            Err(_) => None,
        }
    }
}
