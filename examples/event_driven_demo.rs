// Example of how to use the new event-driven architecture
// This demonstrates the migration path from imperative to event-driven code

use oxidized::{Editor, EventDrivenEditor, EditorEvent, InputEvent, UIEvent, SystemEvent};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🦀 Event-Driven Architecture Demo");

    // Create the traditional editor
    let editor = Editor::new()?;
    
    // Wrap it in the event-driven architecture
    let event_driven = EventDrivenEditor::new(editor);
    
    // Get event sender for sending events
    let sender = event_driven.event_sender();

    println!("Event-driven editor created!");

    // Example: Send some events to demonstrate the system
    
    // 1. Send a key press event
    let key_event = KeyEvent {
        code: KeyCode::Char('j'),
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    };
    
    sender.send(EditorEvent::Input(InputEvent::KeyPress(key_event)))?;
    println!("✓ Sent key press event (j)");

    // 2. Send a redraw request
    sender.send(EditorEvent::UI(UIEvent::RedrawRequest))?;
    println!("✓ Sent redraw request");

    // 3. Send a quit event  
    sender.send(EditorEvent::System(SystemEvent::Quit))?;
    println!("✓ Sent quit event");

    println!("\n🎉 Event-driven architecture demonstration complete!");
    println!("In a full implementation, you would call event_driven.run() to start the event loop.");

    Ok(())
}

// Example of how the event system enables new capabilities:

/// Example plugin that listens for buffer events
pub struct ExamplePlugin {
    name: String,
}

impl ExamplePlugin {
    pub fn new() -> Self {
        Self {
            name: "example".to_string(),
        }
    }
    
    // This would implement EventHandler trait
    pub fn handle_buffer_event(&mut self, event: &EditorEvent) {
        match event {
            EditorEvent::Buffer(buffer_event) => {
                println!("Plugin '{}' received buffer event: {:?}", self.name, buffer_event);
                // Plugin could do things like:
                // - Auto-save on modifications
                // - Trigger linting
                // - Update git status
                // - Send to language server
            }
            _ => {}
        }
    }
}

/// Example of async operations that the event system enables
pub async fn demonstrate_async_capabilities() {
    println!("\n🔄 Async Capabilities Demo:");
    
    // Simulate background syntax highlighting
    tokio::spawn(async {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            println!("  🎨 Background syntax highlighting...");
            // Would send syntax highlight events
            break; // Just for demo
        }
    });
    
    // Simulate LSP integration
    tokio::spawn(async {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            println!("  🔍 LSP diagnostics check...");
            // Would send diagnostic events
            break; // Just for demo
        }
    });
    
    // Simulate auto-save
    tokio::spawn(async {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            println!("  💾 Auto-save check...");
            // Would send save events for modified buffers
            break; // Just for demo
        }
    });
    
    // Let the tasks run briefly
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
    println!("  ✓ Async operations demonstrate non-blocking editor");
}

/// Example of how events enable powerful debugging capabilities
pub fn demonstrate_debugging_features() {
    println!("\n🐛 Debugging Features Demo:");
    
    // Events make it easy to implement:
    println!("  📝 Event logging - all operations are recorded");
    println!("  🔄 Event replay - reproduce bugs exactly");
    println!("  ⏱️  Performance profiling - track event processing time");
    println!("  🔍 State inspection - view editor state at any event");
    println!("  📊 Usage analytics - understand user patterns");
    
    // Example event log entry
    println!("\n  Example event log:");
    println!("    [12:34:56.789] KeyPress(j) -> CursorMoved(1,0 -> 2,0) -> RedrawRequest");
    println!("    [12:34:56.791] Render completed (2.3ms)");
    println!("    [12:34:57.123] KeyPress(i) -> ModeChange(Normal -> Insert)");
}

/// Example of how the event system enables powerful macro recording
pub fn demonstrate_macro_system() {
    println!("\n📹 Macro System Demo:");
    
    println!("  Traditional vim macros record keystrokes");
    println!("  Event-driven macros record semantic operations:");
    println!();
    println!("  Keystroke macro: 'j' 'j' 'i' 'hello' '<Esc>'");
    println!("  Event macro:");
    println!("    1. CursorMoved(0,0 -> 2,0)");
    println!("    2. ModeChange(Normal -> Insert)"); 
    println!("    3. TextInsert('hello', (2,0))");
    println!("    4. ModeChange(Insert -> Normal)");
    println!();
    println!("  Benefits:");
    println!("    ✓ Works across different files");
    println!("    ✓ Adapts to different contexts");
    println!("    ✓ Can be parameterized");
    println!("    ✓ More reliable replay");
}
