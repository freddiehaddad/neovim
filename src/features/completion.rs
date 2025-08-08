/// Command completion system for Vim-style commands
#[derive(Debug, Clone)]
pub struct CommandCompletion {
    /// Available commands for completion
    commands: Vec<CompletionItem>,
    /// Current completion state
    pub active: bool,
    /// Current matches based on input
    pub matches: Vec<CompletionItem>,
    /// Currently selected index
    pub selected_index: usize,
    /// The text that triggered completion
    pub completion_prefix: String,
}

#[derive(Debug, Clone)]
pub struct CompletionItem {
    /// The command text to complete
    pub text: String,
    /// Description of the command
    pub description: String,
    /// Category of command (set, buffer, file, etc.)
    pub category: String,
}

impl CommandCompletion {
    pub fn new() -> Self {
        Self {
            commands: Self::build_command_list(),
            active: false,
            matches: Vec::new(),
            selected_index: 0,
            completion_prefix: String::new(),
        }
    }

    /// Build the complete list of available commands
    fn build_command_list() -> Vec<CompletionItem> {
        let mut commands = Vec::new();

        // Basic ex commands
        commands.extend(vec![
            CompletionItem {
                text: "quit".to_string(),
                description: "Quit editor".to_string(),
                category: "file".to_string(),
            },
            CompletionItem {
                text: "q".to_string(),
                description: "Quit editor (short)".to_string(),
                category: "file".to_string(),
            },
            CompletionItem {
                text: "quit!".to_string(),
                description: "Force quit without saving".to_string(),
                category: "file".to_string(),
            },
            CompletionItem {
                text: "q!".to_string(),
                description: "Force quit without saving (short)".to_string(),
                category: "file".to_string(),
            },
            CompletionItem {
                text: "write".to_string(),
                description: "Save current file".to_string(),
                category: "file".to_string(),
            },
            CompletionItem {
                text: "w".to_string(),
                description: "Save current file (short)".to_string(),
                category: "file".to_string(),
            },
            CompletionItem {
                text: "wq".to_string(),
                description: "Save and quit".to_string(),
                category: "file".to_string(),
            },
            CompletionItem {
                text: "x".to_string(),
                description: "Save and quit (short)".to_string(),
                category: "file".to_string(),
            },
            CompletionItem {
                text: "edit".to_string(),
                description: "Edit file in new buffer".to_string(),
                category: "file".to_string(),
            },
            CompletionItem {
                text: "e".to_string(),
                description: "Edit file in new buffer (short)".to_string(),
                category: "file".to_string(),
            },
        ]);

        // Buffer management commands
        commands.extend(vec![
            CompletionItem {
                text: "buffer".to_string(),
                description: "Switch to buffer".to_string(),
                category: "buffer".to_string(),
            },
            CompletionItem {
                text: "b".to_string(),
                description: "Switch to buffer (short)".to_string(),
                category: "buffer".to_string(),
            },
            CompletionItem {
                text: "bnext".to_string(),
                description: "Switch to next buffer".to_string(),
                category: "buffer".to_string(),
            },
            CompletionItem {
                text: "bn".to_string(),
                description: "Switch to next buffer (short)".to_string(),
                category: "buffer".to_string(),
            },
            CompletionItem {
                text: "bprevious".to_string(),
                description: "Switch to previous buffer".to_string(),
                category: "buffer".to_string(),
            },
            CompletionItem {
                text: "bp".to_string(),
                description: "Switch to previous buffer (short)".to_string(),
                category: "buffer".to_string(),
            },
            CompletionItem {
                text: "bprev".to_string(),
                description: "Switch to previous buffer".to_string(),
                category: "buffer".to_string(),
            },
            CompletionItem {
                text: "bdelete".to_string(),
                description: "Delete current buffer".to_string(),
                category: "buffer".to_string(),
            },
            CompletionItem {
                text: "bd".to_string(),
                description: "Delete current buffer (short)".to_string(),
                category: "buffer".to_string(),
            },
            CompletionItem {
                text: "bdelete!".to_string(),
                description: "Force delete current buffer".to_string(),
                category: "buffer".to_string(),
            },
            CompletionItem {
                text: "bd!".to_string(),
                description: "Force delete current buffer (short)".to_string(),
                category: "buffer".to_string(),
            },
            CompletionItem {
                text: "ls".to_string(),
                description: "List all buffers".to_string(),
                category: "buffer".to_string(),
            },
            CompletionItem {
                text: "buffers".to_string(),
                description: "List all buffers".to_string(),
                category: "buffer".to_string(),
            },
        ]);

        // Window/split commands
        commands.extend(vec![
            CompletionItem {
                text: "split".to_string(),
                description: "Create horizontal split".to_string(),
                category: "window".to_string(),
            },
            CompletionItem {
                text: "sp".to_string(),
                description: "Create horizontal split (short)".to_string(),
                category: "window".to_string(),
            },
            CompletionItem {
                text: "vsplit".to_string(),
                description: "Create vertical split".to_string(),
                category: "window".to_string(),
            },
            CompletionItem {
                text: "vsp".to_string(),
                description: "Create vertical split (short)".to_string(),
                category: "window".to_string(),
            },
            CompletionItem {
                text: "close".to_string(),
                description: "Close current window".to_string(),
                category: "window".to_string(),
            },
        ]);

        // Set commands - display settings
        commands.extend(vec![
            CompletionItem {
                text: "set number".to_string(),
                description: "Show line numbers".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set nu".to_string(),
                description: "Show line numbers (short)".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set nonumber".to_string(),
                description: "Hide line numbers".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set nonu".to_string(),
                description: "Hide line numbers (short)".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set relativenumber".to_string(),
                description: "Show relative line numbers".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set rnu".to_string(),
                description: "Show relative line numbers (short)".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set norelativenumber".to_string(),
                description: "Hide relative line numbers".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set nornu".to_string(),
                description: "Hide relative line numbers (short)".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set cursorline".to_string(),
                description: "Highlight cursor line".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set cul".to_string(),
                description: "Highlight cursor line (short)".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set nocursorline".to_string(),
                description: "Disable cursor line highlight".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set nocul".to_string(),
                description: "Disable cursor line highlight (short)".to_string(),
                category: "set".to_string(),
            },
        ]);

        // Set commands - search and navigation
        commands.extend(vec![
            CompletionItem {
                text: "set ignorecase".to_string(),
                description: "Case-insensitive search".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set ic".to_string(),
                description: "Case-insensitive search (short)".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set noignorecase".to_string(),
                description: "Case-sensitive search".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set noic".to_string(),
                description: "Case-sensitive search (short)".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set smartcase".to_string(),
                description: "Smart case matching".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set scs".to_string(),
                description: "Smart case matching (short)".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set nosmartcase".to_string(),
                description: "Disable smart case".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set noscs".to_string(),
                description: "Disable smart case (short)".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set hlsearch".to_string(),
                description: "Highlight search results".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set hls".to_string(),
                description: "Highlight search results (short)".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set nohlsearch".to_string(),
                description: "Disable search highlighting".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set nohls".to_string(),
                description: "Disable search highlighting (short)".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set incsearch".to_string(),
                description: "Incremental search".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set is".to_string(),
                description: "Incremental search (short)".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set noincsearch".to_string(),
                description: "Disable incremental search".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set nois".to_string(),
                description: "Disable incremental search (short)".to_string(),
                category: "set".to_string(),
            },
        ]);

        // Set commands with values
        commands.extend(vec![
            CompletionItem {
                text: "set tabstop=".to_string(),
                description: "Set tab width".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set ts=".to_string(),
                description: "Set tab width (short)".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set scrolloff=".to_string(),
                description: "Lines to keep around cursor".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set so=".to_string(),
                description: "Lines to keep around cursor (short)".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set sidescrolloff=".to_string(),
                description: "Columns to keep around cursor".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set siso=".to_string(),
                description: "Columns to keep around cursor (short)".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set colorscheme=".to_string(),
                description: "Change color scheme".to_string(),
                category: "set".to_string(),
            },
            CompletionItem {
                text: "set colo=".to_string(),
                description: "Change color scheme (short)".to_string(),
                category: "set".to_string(),
            },
        ]);

        commands
    }

    /// Start completion for the given input
    pub fn start_completion(&mut self, input: &str) {
        self.active = true;
        self.completion_prefix = input.to_string();
        self.update_matches(input);
        self.selected_index = 0;
    }

    /// Update matches based on input
    fn update_matches(&mut self, input: &str) {
        let input_lower = input.to_lowercase();

        self.matches = self
            .commands
            .iter()
            .filter(|cmd| cmd.text.to_lowercase().starts_with(&input_lower))
            .cloned()
            .collect();

        // Sort matches by length (shorter matches first) and then alphabetically
        self.matches.sort_by(|a, b| {
            a.text
                .len()
                .cmp(&b.text.len())
                .then_with(|| a.text.cmp(&b.text))
        });
    }

    /// Move to next completion
    pub fn next(&mut self) {
        if !self.matches.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.matches.len();
        }
    }

    /// Move to previous completion
    pub fn previous(&mut self) {
        if !self.matches.is_empty() {
            self.selected_index = if self.selected_index == 0 {
                self.matches.len() - 1
            } else {
                self.selected_index - 1
            };
        }
    }

    /// Get currently selected completion
    pub fn selected(&self) -> Option<&CompletionItem> {
        self.matches.get(self.selected_index)
    }

    /// Accept current completion and return the completed text
    pub fn accept(&mut self) -> Option<String> {
        if let Some(selected) = self.selected() {
            let completed = selected.text.clone();
            self.cancel();
            Some(completed)
        } else {
            None
        }
    }

    /// Cancel completion
    pub fn cancel(&mut self) {
        self.active = false;
        self.matches.clear();
        self.selected_index = 0;
        self.completion_prefix.clear();
    }

    /// Check if completion menu should be shown
    pub fn should_show(&self) -> bool {
        self.active && !self.matches.is_empty()
    }

    /// Get visible completion items (for rendering)
    pub fn visible_items(&self, max_items: usize) -> &[CompletionItem] {
        if self.matches.is_empty() {
            return &[];
        }

        let start_idx = if self.matches.len() <= max_items {
            0
        } else {
            // Center the selected item in the visible window
            let half_visible = max_items / 2;
            if self.selected_index < half_visible {
                0
            } else if self.selected_index >= self.matches.len() - half_visible {
                self.matches.len() - max_items
            } else {
                self.selected_index - half_visible
            }
        };

        let end_idx = (start_idx + max_items).min(self.matches.len());
        &self.matches[start_idx..end_idx]
    }

    /// Get the relative index of the selected item in the visible window
    pub fn visible_selected_index(&self, max_items: usize) -> usize {
        let visible_items = self.visible_items(max_items);
        if visible_items.is_empty() {
            return 0;
        }

        // Find the selected item in the visible window
        for (i, item) in visible_items.iter().enumerate() {
            if let Some(selected) = self.selected() {
                if item.text == selected.text {
                    return i;
                }
            }
        }
        0
    }
}

impl Default for CommandCompletion {
    fn default() -> Self {
        Self::new()
    }
}
