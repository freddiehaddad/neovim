use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use oxidized::features::macros::*;

fn create_key_event(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn test_macro_recorder_creation() {
    let recorder = MacroRecorder::new();
    assert!(!recorder.is_recording());
    assert_eq!(recorder.recording_register(), None);
    assert_eq!(recorder.list_macros().len(), 0);
}

#[test]
fn test_start_stop_recording() {
    let mut recorder = MacroRecorder::new();

    // Start recording
    assert!(recorder.start_recording('a').is_ok());
    assert!(recorder.is_recording());
    assert_eq!(recorder.recording_register(), Some('a'));

    // Can't start recording again
    assert!(recorder.start_recording('b').is_err());

    // Stop recording
    let register = recorder.stop_recording().unwrap();
    assert_eq!(register, 'a');
    assert!(!recorder.is_recording());
    assert!(recorder.has_macro('a'));
}

#[test]
fn test_record_events() {
    let mut recorder = MacroRecorder::new();
    recorder.start_recording('a').unwrap();

    // Record some events
    recorder.record_event(create_key_event(KeyCode::Char('i')));
    recorder.record_event(create_key_event(KeyCode::Char('h')));
    recorder.record_event(create_key_event(KeyCode::Char('e')));
    recorder.record_event(create_key_event(KeyCode::Esc));

    recorder.stop_recording().unwrap();

    let macro_rec = recorder.get_macro('a').unwrap();
    assert_eq!(macro_rec.len(), 4);
}

#[test]
fn test_play_macro() {
    let mut recorder = MacroRecorder::new();
    recorder.start_recording('a').unwrap();

    recorder.record_event(create_key_event(KeyCode::Char('i')));
    recorder.record_event(create_key_event(KeyCode::Char('h')));
    recorder.stop_recording().unwrap();

    // Check macro was recorded
    let macro_rec = recorder.get_macro('a').unwrap();
    assert_eq!(macro_rec.len(), 2);

    let events = recorder.play_macro('a').unwrap();
    assert_eq!(events.len(), 2);

    recorder.finish_playback();

    // Check last played is set after finish_playback
    let last_events = recorder.play_last_macro().unwrap();
    assert_eq!(last_events.len(), 2);

    recorder.finish_playback();
}

#[test]
fn test_invalid_operations() {
    let mut recorder = MacroRecorder::new();

    // Invalid register
    assert!(recorder.start_recording('!').is_err());

    // Stop without recording
    assert!(recorder.stop_recording().is_err());

    // Play non-existent macro
    assert!(recorder.play_macro('z').is_err());

    // Play last without previous
    assert!(recorder.play_last_macro().is_err());
}

#[test]
fn test_macro_management() {
    let mut recorder = MacroRecorder::new();
    recorder.start_recording('a').unwrap();
    recorder.record_event(create_key_event(KeyCode::Char('x')));
    recorder.stop_recording().unwrap();

    recorder.start_recording('b').unwrap();
    recorder.record_event(create_key_event(KeyCode::Char('y')));
    recorder.stop_recording().unwrap();

    assert_eq!(recorder.list_macros().len(), 2);

    assert!(recorder.clear_macro('a'));
    assert!(!recorder.clear_macro('z'));
    assert_eq!(recorder.list_macros().len(), 1);

    recorder.clear_all_macros();
    assert_eq!(recorder.list_macros().len(), 0);
}

#[test]
fn test_prevent_recording_q_command() {
    let mut recorder = MacroRecorder::new();
    recorder.start_recording('a').unwrap();

    // Record some events including 'q' (should be filtered out)
    recorder.record_event(create_key_event(KeyCode::Char('i')));
    recorder.record_event(create_key_event(KeyCode::Char('q'))); // Should be ignored
    recorder.record_event(create_key_event(KeyCode::Char('x')));

    recorder.stop_recording().unwrap();

    let macro_rec = recorder.get_macro('a').unwrap();
    assert_eq!(macro_rec.len(), 2); // 'q' should be filtered out
}

#[test]
fn test_macro_key_event_conversion() {
    let original = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
    let macro_event = MacroKeyEvent::from(original);
    let restored = KeyEvent::from(macro_event);

    assert_eq!(original.code, restored.code);
    assert_eq!(original.modifiers, restored.modifiers);
}

#[test]
fn test_macro_structure() {
    let mut macro_def = Macro::new('a');

    assert_eq!(macro_def.name, 'a');
    assert!(macro_def.is_empty());
    assert_eq!(macro_def.len(), 0);

    macro_def.add_event(create_key_event(KeyCode::Char('i')));
    macro_def.add_event(create_key_event(KeyCode::Char('h')));

    assert!(!macro_def.is_empty());
    assert_eq!(macro_def.len(), 2);
}

#[test]
fn test_macro_status() {
    let recorder = MacroRecorder::new();
    let status = recorder.get_status();
    assert!(status.contains("Not recording"));

    let mut recorder = MacroRecorder::new();
    recorder.start_recording('a').unwrap();
    let status = recorder.get_status();
    assert!(status.contains("Recording"));
}

#[test]
fn test_last_played_register() {
    let mut recorder = MacroRecorder::new();

    // No last played initially
    assert_eq!(recorder.get_last_played_register(), None);

    // Record and play a macro
    recorder.start_recording('a').unwrap();
    recorder.record_event(create_key_event(KeyCode::Char('x')));
    recorder.stop_recording().unwrap();

    recorder.play_macro('a').unwrap();
    assert_eq!(recorder.get_last_played_register(), Some('a'));
}
