use crate::editor::Editor;
use anyhow::Result;
use log::{debug, info, warn};
use std::sync::{Arc, Mutex};

/// Event-driven editor that replaces the traditional main loop
pub struct EventDrivenEditor {
    editor: Arc<Mutex<Editor>>,
    should_quit: Arc<Mutex<bool>>,
}

impl EventDrivenEditor {
    /// Create a new event-driven editor
    pub fn new(editor: Editor) -> Self {
        Self {
            editor: Arc::new(Mutex::new(editor)),
            should_quit: Arc::new(Mutex::new(false)),
        }
    }

    /// Main event loop - replaces the traditional Editor::run() method
    pub fn run(&mut self) -> Result<(), anyhow::Error> {
        info!("Starting event-driven editor");

        // Initialize editor if no buffers exist and do initial render
        if let Ok(mut editor) = self.editor.lock() {
            if editor.current_buffer().is_none() {
                debug!("No buffers exist, creating initial empty buffer");
                if let Err(e) = editor.create_buffer(None) {
                    warn!("Failed to create initial buffer: {}", e);
                }
            }

            // Initial render (similar to original Editor::run())
            if let Err(e) = editor.render() {
                warn!("Initial render failed: {}", e);
            }
        }

        loop {
            // Check quit condition
            if let Ok(quit) = self.should_quit.lock() {
                if *quit {
                    info!("Quit requested, exiting event loop");
                    break;
                }
            }

            // Use the editor's existing handle_input method directly
            // This preserves all the existing functionality
            if let Ok(mut editor) = self.editor.lock() {
                match editor.handle_input() {
                    Ok(input_processed) => {
                        if input_processed {
                            // Re-render if input was processed (same as original Editor::run())
                            if let Err(e) = editor.render() {
                                warn!("Render failed: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Input handling failed: {}", e);
                    }
                }

                // Check if editor wants to quit
                if editor.should_quit() {
                    info!("Editor requested quit, exiting event loop");
                    break;
                }
            } else {
                warn!("Failed to lock editor");
                break;
            }

            // Small delay to prevent busy waiting (like original Editor::run())
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        info!("Event-driven editor loop completed");
        Ok(())
    }

    /// Signal that the editor should quit
    pub fn quit(&self) {
        if let Ok(mut quit) = self.should_quit.lock() {
            *quit = true;
        }
    }
}
