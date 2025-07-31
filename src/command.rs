// Command parsing and execution
// This will handle Ex commands (:w, :q, etc.)

pub struct Command {
    pub name: String,
    pub args: Vec<String>,
}

impl Command {
    pub fn parse(input: &str) -> Option<Self> {
        let parts: Vec<&str> = input.trim().split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }

        let name = parts[0].to_string();
        let args = parts[1..].iter().map(|s| s.to_string()).collect();

        Some(Self { name, args })
    }
}

// TODO: Implement command registry and execution engine
