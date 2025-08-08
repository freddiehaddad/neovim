use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use oxidized::input::events::{BufferEvent, EditorEvent, InputEvent, SystemEvent, UIEvent};

#[test]
fn test_event_macros() {
    let key_event = KeyEvent {
        code: KeyCode::Char('a'),
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    };

    // Test that the event! macro creates the correct event types
    let input_event = oxidized::event!(input::key_press(key_event));
    let buffer_event = oxidized::event!(buffer::modified(1));
    let ui_event = oxidized::event!(ui::redraw);
    let system_event = oxidized::event!(system::quit);

    // Verify the events have the correct structure
    match input_event {
        EditorEvent::Input(InputEvent::KeyPress(_)) => {}
        _ => panic!("Expected KeyPress event"),
    }

    match buffer_event {
        EditorEvent::Buffer(BufferEvent::Modified { buffer_id: 1 }) => {}
        _ => panic!("Expected Buffer Modified event with id 1"),
    }

    match ui_event {
        EditorEvent::UI(UIEvent::RedrawRequest) => {}
        _ => panic!("Expected UI RedrawRequest event"),
    }

    match system_event {
        EditorEvent::System(SystemEvent::Quit) => {}
        _ => panic!("Expected System Quit event"),
    }
}
