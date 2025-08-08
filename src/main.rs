use anyhow::Result;
use oxidized::{Editor, EventDrivenEditor};
use std::env;
use std::path::PathBuf;

fn main() -> Result<()> {
    // Initialize comprehensive logging
    use env_logger::{Builder, Target};
    use std::io::Write;

    // Configure logging based on build type with timestamps and better formatting
    let mut builder = if cfg!(debug_assertions) {
        // Debug builds: Enable trace logging by default with detailed formatting
        Builder::from_env(env_logger::Env::default().default_filter_or("debug"))
    } else {
        // Release builds: Use default behavior (respects RUST_LOG environment variable)
        Builder::from_default_env()
    };

    // Enhanced log formatting with timestamps and module names
    builder
        .format(|buf, record| {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            writeln!(
                buf,
                "{} [{}] [{}:{}] {}",
                timestamp,
                record.level(),
                record.module_path().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                record.args()
            )
        })
        .target(Target::Pipe(Box::new(std::fs::File::create(
            "oxidized.log",
        )?)))
        .init();

    log::info!("=== Oxidized Text Editor Starting ===");
    log::info!(
        "Build type: {}",
        if cfg!(debug_assertions) {
            "DEBUG"
        } else {
            "RELEASE"
        }
    );
    log::info!("Version: {}", env!("CARGO_PKG_VERSION"));
    log::debug!(
        "Working directory: {:?}",
        std::env::current_dir().unwrap_or_default()
    );

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

    // Create event-driven editor and run it
    let mut event_driven_editor = EventDrivenEditor::new(editor);
    if let Err(e) = event_driven_editor.run() {
        eprintln!("Editor error: {}", e);
        std::process::exit(1);
    }

    Ok(())
}
