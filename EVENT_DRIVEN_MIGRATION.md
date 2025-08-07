# Event-Driven Architecture Migration Guide

This document outlines the plan to migrate the Oxidized editor from its current imperative main loop to an event-driven architecture similar to Neovim and Vim.

## Current Architecture Problems

### 1. Monolithic Main Loop

The current `Editor::run()` method is a blocking loop that:

- Polls for input continuously
- Renders on every iteration (even when nothing changed)
- Couples input handling, rendering, and state management
- Makes it difficult to add asynchronous operations
- Prevents clean separation of concerns

```rust
// Current problematic pattern in editor.rs
pub fn run(&mut self) -> Result<()> {
    loop {
        if self.should_quit { break; }
        let input_handled = self.handle_input()?;  // Blocking
        if input_handled {
            self.render()?;  // Always renders after input
        }
    }
}
```

### 2. Tight Coupling

- UI directly calls editor methods
- Editor directly manipulates buffers
- No clear separation between input, state, and rendering
- Hard to test individual components
- Difficult to extend with plugins

### 3. Synchronous Operations

- File I/O blocks the entire editor
- Syntax highlighting happens on the main thread
- LSP operations would block the UI
- No ability to handle multiple concurrent operations

## Event-Driven Solution

### Architecture Overview

```text
┌─────────────────┐    Events     ┌─────────────────┐
│  Input Sources  │──────────────→│   Event Bus     │
└─────────────────┘               └─────────────────┘
                                           │
                                           ▼
┌─────────────────┐               ┌─────────────────┐
│   Event Loop    │◄──────────────│  Event Router   │
└─────────────────┘               └─────────────────┘
         │                                 │
         ▼                                 ▼
┌─────────────────┐               ┌─────────────────┐
│   Renderer      │               │ Event Handlers  │
└─────────────────┘               └─────────────────┘
                                           │
                                           ▼
                                 ┌─────────────────┐
                                 │ Editor State    │
                                 └─────────────────┘
```

### Core Event Types

1. **Input Events**
   - Raw key presses
   - Parsed key sequences (gg, dd, 2yy)
   - Mode transitions
   - Commands

2. **Buffer Events**
   - Content changes
   - Cursor movements
   - File operations
   - Syntax highlighting updates

3. **UI Events**
   - Redraw requests
   - Theme changes
   - Status updates
   - Window resizing

4. **Window Events**
   - Split operations
   - Focus changes
   - Layout updates

5. **System Events**
   - File system changes
   - Configuration updates
   - Quit requests

## Migration Plan

### Phase 1: Event System Foundation ✅

#### Status: Complete

- [x] Created `src/event.rs` with comprehensive event types
- [x] Implemented `EventBus` and `EventDispatcher`
- [x] Created `EventHandler` trait
- [x] Added event macros for convenience

**Files Added:**

- `src/event.rs` - Core event system
- `src/event_driven.rs` - Event handlers and main loop

### Phase 2: Handler Implementation ✅

#### Status: Complete

- [x] `InputEventHandler` - Processes keyboard input
- [x] `BufferEventHandler` - Manages buffer operations  
- [x] `UIEventHandler` - Handles rendering and display
- [x] `ConfigEventHandler` - Configuration changes
- [x] `SystemEventHandler` - System-level events

### Phase 3: Core Editor Refactoring

### Status: TODO

#### Step 1: Extract State Management

```rust
// Create separate state struct
pub struct EditorState {
    buffers: HashMap<usize, Buffer>,
    current_buffer_id: Option<usize>,
    mode: Mode,
    config: EditorConfig,
    // ... other state
}

impl EditorState {
    pub fn apply_event(&mut self, event: &EditorEvent) -> Vec<EditorEvent> {
        // Pure state transitions
    }
}
```

#### Step 2: Refactor Editor Methods

- Convert imperative methods to event generators
- Separate state changes from side effects
- Make operations atomic and reversible

```rust
// Old imperative style:
impl Editor {
    pub fn insert_char(&mut self, c: char) {
        // Directly modifies state
        self.current_buffer_mut().insert_char(c);
        self.mark_buffer_modified();
        self.trigger_syntax_highlight();
    }
}

// New event-driven style:
impl Editor {
    pub fn insert_char(&self, c: char) -> Vec<EditorEvent> {
        vec![
            EditorEvent::Buffer(BufferEvent::ContentChanged { ... }),
            EditorEvent::Buffer(BufferEvent::Modified { ... }),
            EditorEvent::UI(UIEvent::RedrawRequest),
        ]
    }
}
```

#### Step 3: Update UI Layer

- Make UI purely reactive to events
- Remove direct editor coupling
- Implement efficient differential rendering

### Phase 4: Input System Rewrite

#### Status: TODO

**Key Changes:**

- Replace direct key handling with event emission
- Implement key sequence parsing as event transformation
- Add macro recording/replay as event capture/replay
- Support multi-stage operations (operator + text object)

```rust
// Current key handler becomes event emitter
impl KeyHandler {
    pub fn handle_key(&self, key: KeyEvent, mode: Mode) -> Vec<EditorEvent> {
        match mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::Insert => self.handle_insert_key(key),
            // ...
        }
    }
}
```

### Phase 5: Asynchronous Operations

#### Status: TODO

**Async Components to Add:**

- Background syntax highlighting
- File I/O operations
- LSP client integration
- Plugin system
- Auto-save functionality

```rust
// Async event handler example
#[async_trait]
pub trait AsyncEventHandler {
    async fn handle_async_event(&mut self, event: &EditorEvent) -> EventResult;
}

pub struct AsyncSyntaxHandler {
    highlighter: AsyncSyntaxHighlighter,
}

impl AsyncEventHandler for AsyncSyntaxHandler {
    async fn handle_async_event(&mut self, event: &EditorEvent) -> EventResult {
        match event {
            EditorEvent::Buffer(BufferEvent::ContentChanged { buffer_id, .. }) => {
                let highlights = self.highlighter.highlight_async(*buffer_id).await;
                EventResult::Events(vec![
                    EditorEvent::Buffer(BufferEvent::SyntaxHighlighted { 
                        buffer_id: *buffer_id,
                        highlights 
                    })
                ])
            }
            _ => EventResult::NotHandled,
        }
    }
}
```

### Phase 6: Plugin System Integration

#### Status: Future

**Plugin Architecture:**

- Plugins register event handlers
- Plugins can emit custom events
- Sandboxed plugin execution
- Hot plugin reloading

```rust
pub trait Plugin {
    fn name(&self) -> &str;
    fn handlers(&self) -> Vec<Box<dyn EventHandler>>;
    fn handle_custom_event(&mut self, event: &PluginEvent) -> EventResult;
}
```

### Phase 7: LSP Integration

#### Status: Future

**LSP Event Types:**

- Diagnostics events
- Completion events  
- Hover events
- Go-to-definition events
- Code action events

## Implementation Strategy

### 1. Gradual Migration

- Keep both old and new systems running in parallel
- Add feature flags to toggle between implementations
- Migrate one subsystem at a time

### 2. Backward Compatibility

- Maintain existing Editor API during transition
- Create adapter layer for old code
- Incremental testing and validation

### 3. Testing Strategy

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_flow() {
        let mut editor = EventDrivenEditor::new();
        
        // Send input event
        editor.send_event(event!(input::key_press(key_j)));
        
        // Process events
        let events = editor.process_events();
        
        // Verify cursor movement event was generated
        assert!(events.iter().any(|e| matches!(e, 
            EditorEvent::Buffer(BufferEvent::CursorMoved { .. })
        )));
    }
}
```

## Benefits of Event-Driven Architecture

### 1. **Decoupling**

- Components communicate only through events
- Easy to swap implementations
- Clear dependency boundaries
- Testable in isolation

### 2. **Extensibility**

- Plugins add event handlers
- New features don't modify core
- Event middleware for cross-cutting concerns
- Hot-pluggable components

### 3. **Asynchronous Operations**

- Non-blocking file I/O
- Background syntax highlighting
- Concurrent LSP operations
- Responsive UI during heavy operations

### 4. **Debugging and Introspection**

- Complete event history
- Reproducible bugs via event replay
- Performance profiling by event type
- Live debugging with event inspection

### 5. **Undo/Redo System**

- Events are naturally undoable
- Fine-grained operation history
- Atomic composite operations
- Efficient change tracking

### 6. **Macro System**

- Record events instead of keystrokes
- Replay complex operations
- Parameterized macros
- Cross-buffer operations

## Performance Considerations

### Event Overhead

- Use Arc<> for shared data in events
- Pool events to reduce allocations
- Batch related events together
- Lazy event evaluation where possible

### Memory Usage

- Bounded event queues
- Efficient event serialization
- Garbage collection of old events
- Smart pointer usage

### Rendering Efficiency

- Coalesce redraw requests
- Differential rendering
- Dirty region tracking
- Frame rate limiting

## Migration Timeline

**Week 1-2:** Phase 1 & 2 (Event System Foundation) ✅
**Week 3-4:** Phase 3 (Core Editor Refactoring)
**Week 5-6:** Phase 4 (Input System Rewrite)  
**Week 7-8:** Phase 5 (Async Operations)
**Week 9+:** Phase 6 & 7 (Plugins & LSP)

## Example: Converting Insert Mode

### Before (Imperative)

```rust
// In handle_input()
KeyCode::Char(c) if mode == Mode::Insert => {
    self.current_buffer_mut().insert_char(c);
    self.move_cursor_right();
    self.mark_buffer_modified();
    self.request_syntax_highlight();
    self.render()?;
}
```

### After (Event-Driven)

```rust
// In InputEventHandler
KeyCode::Char(c) if mode == Mode::Insert => {
    vec![
        EditorEvent::Input(InputEvent::TextInsert {
            text: c.to_string(),
            position: cursor_pos,
        }),
        EditorEvent::Buffer(BufferEvent::CursorMoved {
            buffer_id,
            old_pos: cursor_pos,
            new_pos: (cursor_pos.0, cursor_pos.1 + 1),
        }),
        EditorEvent::Buffer(BufferEvent::Modified { buffer_id }),
        EditorEvent::UI(UIEvent::RedrawRequest),
    ]
}
```

## Conclusion

The event-driven architecture provides a solid foundation for:

- Plugin development
- LSP integration  
- Asynchronous operations
- Advanced features (macros, undo/redo)
- Better testing and debugging
- More maintainable code

This migration transforms Oxidized from a traditional text editor into a modern, extensible, and responsive editing platform that matches the capabilities of Neovim and Vim.
