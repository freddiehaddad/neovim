/// Macro Recording and Playback System
///
/// This module implements Vim-style macro functionality:
/// - q{register}: Start/stop macro recording
/// - @{register}: Playback macro  
/// - @@: Repeat last macro
/// - {count}@{register}: Repeat macro count times
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use log::{debug, error, info, warn};
use std::collections::HashMap;
use thiserror::Error;

/// Errors that can occur during macro operations
#[derive(Error, Debug)]
pub enum MacroError {
    #[error("Invalid register '{0}'. Must be a-z, A-Z, or 0-9")]
    InvalidRegister(char),
    #[error("No macro recorded in register '{0}'")]
    MacroNotFound(char),
    #[error("Cannot record macro while already recording")]
    AlreadyRecording,
    #[error("No macro currently being recorded")]
    NotRecording,
    #[error("Macro playback failed: {0}")]
    PlaybackFailed(String),
}

/// Represents a recorded key event in a macro
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MacroKeyEvent {
    pub key_code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl From<KeyEvent> for MacroKeyEvent {
    fn from(key_event: KeyEvent) -> Self {
        Self {
            key_code: key_event.code,
            modifiers: key_event.modifiers,
        }
    }
}

impl From<MacroKeyEvent> for KeyEvent {
    fn from(macro_key: MacroKeyEvent) -> Self {
        KeyEvent::new(macro_key.key_code, macro_key.modifiers)
    }
}

/// A recorded macro containing a sequence of key events
#[derive(Debug, Clone)]
pub struct Macro {
    pub name: char,
    pub events: Vec<MacroKeyEvent>,
    pub description: String,
}

impl Macro {
    pub fn new(name: char) -> Self {
        Self {
            name,
            events: Vec::new(),
            description: format!("Macro {}", name),
        }
    }

    pub fn add_event(&mut self, event: KeyEvent) {
        // Don't record the macro stop command (q) to prevent infinite loops
        if let KeyCode::Char('q') = event.code {
            if event.modifiers.is_empty() {
                debug!("Skipping macro stop command in recording");
                return;
            }
        }

        self.events.push(event.into());
        debug!("Added event to macro '{}': {:?}", self.name, event);
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }
}

/// Main macro recording and playback system
#[derive(Debug)]
pub struct MacroRecorder {
    /// Currently recording macro
    recording: Option<Macro>,
    /// Stored macros by register (a-z, A-Z, 0-9)
    macros: HashMap<char, Macro>,
    /// Last played macro register for @@ command
    last_played: Option<char>,
    /// Whether we're currently playing back a macro (prevents infinite loops)
    playing: bool,
}

impl Default for MacroRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl MacroRecorder {
    /// Create a new macro recorder
    pub fn new() -> Self {
        info!("Initializing macro recorder");
        Self {
            recording: None,
            macros: HashMap::new(),
            last_played: None,
            playing: false,
        }
    }

    /// Check if a register name is valid (a-z, A-Z, 0-9)
    fn is_valid_register(register: char) -> bool {
        register.is_ascii_alphanumeric()
    }

    /// Start recording a macro to the specified register
    pub fn start_recording(&mut self, register: char) -> Result<(), MacroError> {
        if !Self::is_valid_register(register) {
            return Err(MacroError::InvalidRegister(register));
        }

        if self.recording.is_some() {
            return Err(MacroError::AlreadyRecording);
        }

        if self.playing {
            warn!("Attempting to record macro while playing back - ignoring");
            return Ok(());
        }

        let macro_record = Macro::new(register);
        self.recording = Some(macro_record);

        info!("Started recording macro to register '{}'", register);
        Ok(())
    }

    /// Stop recording the current macro
    pub fn stop_recording(&mut self) -> Result<char, MacroError> {
        let recorded_macro = self.recording.take().ok_or(MacroError::NotRecording)?;

        let register = recorded_macro.name;
        let event_count = recorded_macro.len();

        // Store the macro even if it's empty (Vim behavior)
        self.macros.insert(register, recorded_macro);

        info!(
            "Stopped recording macro '{}' with {} events",
            register, event_count
        );
        Ok(register)
    }

    /// Add a key event to the currently recording macro
    pub fn record_event(&mut self, event: KeyEvent) {
        if let Some(ref mut macro_record) = self.recording {
            macro_record.add_event(event);
        }
    }

    /// Check if currently recording a macro
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Get the register of the currently recording macro
    pub fn recording_register(&self) -> Option<char> {
        self.recording.as_ref().map(|m| m.name)
    }

    /// Check if a macro exists in the specified register
    pub fn has_macro(&self, register: char) -> bool {
        self.macros.contains_key(&register)
    }

    /// Get a macro by register
    pub fn get_macro(&self, register: char) -> Option<&Macro> {
        self.macros.get(&register)
    }

    /// Play back a macro from the specified register
    pub fn play_macro(&mut self, register: char) -> Result<Vec<KeyEvent>, MacroError> {
        if !Self::is_valid_register(register) {
            return Err(MacroError::InvalidRegister(register));
        }

        let macro_record = self
            .macros
            .get(&register)
            .ok_or(MacroError::MacroNotFound(register))?;

        if self.playing {
            warn!("Already playing a macro - preventing recursion");
            return Ok(Vec::new());
        }

        self.playing = true;
        self.last_played = Some(register);

        let events: Vec<KeyEvent> = macro_record.events.iter().map(|me| (*me).into()).collect();

        info!("Playing macro '{}' with {} events", register, events.len());

        // Note: We set playing back to false after the macro is executed
        // This will be handled by the caller

        Ok(events)
    }

    /// Play back the last executed macro (@@ command)
    pub fn play_last_macro(&mut self) -> Result<Vec<KeyEvent>, MacroError> {
        let last_register = self
            .last_played
            .ok_or_else(|| MacroError::PlaybackFailed("No previous macro to repeat".to_string()))?;

        self.play_macro(last_register)
    }

    /// Get the last played macro register
    pub fn get_last_played_register(&self) -> Option<char> {
        self.last_played
    }

    /// Mark macro playback as finished (called after executing events)
    pub fn finish_playback(&mut self) {
        self.playing = false;
        debug!("Macro playback finished");
    }

    /// Get list of all recorded macros
    pub fn list_macros(&self) -> Vec<(char, &Macro)> {
        let mut macros: Vec<_> = self
            .macros
            .iter()
            .map(|(&reg, macro_rec)| (reg, macro_rec))
            .collect();
        macros.sort_by_key(|&(reg, _)| reg);
        macros
    }

    /// Clear a specific macro
    pub fn clear_macro(&mut self, register: char) -> bool {
        if self.macros.remove(&register).is_some() {
            info!("Cleared macro in register '{}'", register);
            true
        } else {
            false
        }
    }

    /// Clear all macros
    pub fn clear_all_macros(&mut self) {
        let count = self.macros.len();
        self.macros.clear();
        info!("Cleared {} macros", count);
    }

    /// Get macro recording status for display
    pub fn get_status(&self) -> String {
        if let Some(ref macro_record) = self.recording {
            format!(
                "Recording macro '{}' ({} events)",
                macro_record.name,
                macro_record.len()
            )
        } else {
            "Not recording macro".to_string()
        }
    }
}
