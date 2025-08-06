use anyhow::Result;
use oxidized::Editor;
use std::env;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging - logging to file by default
    use env_logger::Target;

    // Configure logging based on build type
    let mut builder = if cfg!(debug_assertions) {
        // Debug builds: Enable trace logging by default
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("trace"))
    } else {
        // Release builds: Use default behavior (respects RUST_LOG environment variable)
        env_logger::Builder::from_default_env()
    };

    builder
        .target(Target::Pipe(Box::new(std::fs::File::create(
            "oxidized.log",
        )?)))
        .init();

    // Option: Default stderr logging (uncomment to use instead)
    // env_logger::init();

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
