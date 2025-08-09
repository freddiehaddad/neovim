// Command parsing and execution
// This will handle Ex commands (:w, :q, etc.)

use log::{debug, warn};

pub struct Command {
    pub name: String,
    pub args: Vec<String>,
}

impl Command {
    pub fn parse(input: &str) -> Option<Self> {
        debug!("Parsing command: '{}'", input);
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.is_empty() {
            warn!("Empty command input received");
            return None;
        }

        let name = parts[0].to_string();
        let args: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
        debug!(
            "Parsed command '{}' with {} args: {:?}",
            name,
            args.len(),
            args
        );

        Some(Self { name, args })
    }
}

// TODO: Implement command registry and execution engine
