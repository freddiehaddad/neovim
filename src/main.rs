use anyhow::Result;
use neovim::Editor;
use std::env;
use std::path::PathBuf;

fn main() -> Result<()> {
    // Initialize logging
    env_logger::init();

    // Create editor instance
    let mut editor = Editor::new()?;

    // Check for command line arguments (file to open)
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 {
        let file_path = PathBuf::from(&args[1]);
        if let Err(e) = editor.create_buffer(Some(file_path)) {
            eprintln!("Error opening file {}: {}", args[1], e);
        }
    }

    // Run the editor
    editor.run()?;

    Ok(())
}
