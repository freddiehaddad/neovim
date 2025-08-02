use anyhow::Result;
use neovim::Editor;
use std::env;
use std::path::PathBuf;

fn main() -> Result<()> {
    // Initialize logging - you can uncomment one of these options:

    // Option 1: Default stderr logging (current)
    env_logger::init();

    // Option 2: Log to file (uncomment to use)
    // use env_logger::Target;
    // env_logger::Builder::from_default_env()
    //     .target(Target::Pipe(Box::new(std::fs::File::create("neovim.log")?)))
    //     .init();

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
