# Neovim Clone

A revolutionary Vim/Neovim clone written in Rust that breaks from traditional Vim with a **TOML-first configuration system** for both keymaps and editor settings.

## 🚀 Revolutionary Features

### ⚡ TOML-Based Configuration System

Unlike traditional Vim, this editor uses **human-readable TOML files** for all configuration:

- **`keymaps.toml`** - Complete keymap customization
- **`editor.toml`** - Comprehensive editor settings (30+ options)
- **No more scattered hardcoded defaults** - everything is configurable
- **Persistent settings** - all `:set` commands automatically save to configuration

### 🎯 Familiar Vim Interface with Modern Backend

- Full Vim command compatibility (`:set number`, `:set relativenumber`, etc.)
- TOML configuration with Vim-style aliases (`number`/`nu`, `relativenumber`/`rnu`)
- Automatic configuration loading and UI synchronization

## Current Features ✅

### Core Editor Functionality

- **Modal Editing**: Normal, Insert, Command, Visual, Replace, and Search modes
- **TOML-based Keymap System**: Configurable keybindings loaded from `keymaps.toml`
- **TOML-based Editor Settings**: Comprehensive configuration in `editor.toml`
- **Key Sequence Support**: Multi-character commands like `dd`, `gg`, `yy` with timeout
- **Buffer Management**: Multiple buffer support with undo/redo
- **Copy/Paste System**: Vim-compatible yank/put operations with type awareness
- **Search Engine**: Regex-capable search with `/`, `n`, `N` navigation
- **Line Numbers**: Absolute, relative, and hybrid line number modes
- **Cursor Line Highlighting**: Visual cursor line highlighting
- **Basic Text Operations**: Insert, delete, line breaks, word movement
- **Cursor Movement**: hjkl movement, arrow keys, word navigation, line/buffer navigation
- **Terminal Interface**: Raw terminal input/output with crossterm
- **Status Line**: Shows current mode, file info, cursor position

### Vim Keybindings

- **Insert Mode**: `i`, `I`, `a`, `A`, `o`, `O`
- **Navigation**: `h/j/k/l`, arrow keys, `w/b/e` (word movement), `0/$` (line start/end), `gg/G` (buffer start/end)
- **Mode Switching**: `Esc`, `:`, `/`, `v`, `V`, `R` (replace mode)
- **Delete Operations**: `x` (delete char), `X` (delete char before), `dd` (delete line)
- **Copy/Paste**: `yy` (yank line), `yw` (yank word), `y$` (yank to end), `p/P` (put after/before)
- **Undo/Redo**: `u`, `Ctrl+r`
- **File Operations**: `:w`, `:q`, `:q!`, `:wq`, `:e filename`

### Ex Commands

- `:q` - Quit
- `:q!` - Force quit
- `:w` - Save file
- `:wq` - Save and quit
- `:e <filename>` - Open file

### Search Commands

- `/pattern` - Search forward for pattern (supports regex)
- `n` - Go to next search result
- `N` - Go to previous search result

### Configuration Commands

- `:set number` - Enable absolute line numbers
- `:set nonumber` - Disable absolute line numbers
- `:set relativenumber` - Enable relative line numbers
- `:set norelativenumber` - Disable relative line numbers
- `:set cursorline` - Enable cursor line highlighting
- `:set nocursorline` - Disable cursor line highlighting
- `:set tabstop=4` - Set tab width to 4 spaces

## Usage

```bash
# Build the project
cargo build

# Run without opening a file
cargo run

# Open a specific file
cargo run -- filename.txt
```

### Quick Start Guide

1. **Start the editor**: `cargo run`
2. **Enter Insert mode**: Press `i` to start typing
3. **Return to Normal mode**: Press `Esc`
4. **Navigate**: Use `hjkl` or arrow keys
5. **Search**: Type `/pattern` to search, `n` for next result
6. **Configure**: Use `:set number` to show line numbers
7. **Save and quit**: Type `:wq`

### Configuration Examples

```bash
# Enable line numbers and cursor highlighting
:set number
:set cursorline

# Use relative line numbers for easier navigation
:set relativenumber

# Set tab width to 8 spaces
:set tabstop=8

# All settings are automatically saved to editor.toml
```

### Basic Usage

1. Start in Normal mode
2. Press `i` to enter Insert mode and type text
3. Press `Esc` to return to Normal mode
4. Use `hjkl` or arrow keys to move cursor
5. Type `:w` to save, `:q` to quit

### Configuration

The editor features a **revolutionary TOML-based configuration system** that completely replaces traditional Vim's scattered settings approach.

#### Editor Configuration (`editor.toml`)

The `editor.toml` file contains comprehensive editor settings organized into logical sections:

```toml
[display]
show_line_numbers = true      # :set number / :set nonumber
show_relative_numbers = false # :set relativenumber / :set norelativenumber
show_cursor_line = false      # :set cursorline / :set nocursorline
color_scheme = "default"      # Color scheme to use
syntax_highlighting = true    # Enable syntax highlighting

[behavior]
tab_width = 4                 # Width of tab characters (:set tabstop)
expand_tabs = false           # Use spaces instead of tabs (:set expandtab)
auto_indent = true            # Automatic indentation
ignore_case = false           # Ignore case in searches (:set ignorecase)
smart_case = false            # Smart case searching (:set smartcase)
highlight_search = true       # Highlight search results (:set hlsearch)
incremental_search = true     # Incremental search (:set incsearch)
wrap_lines = false            # Wrap long lines (:set wrap)

[editing]
undo_levels = 1000            # Number of undo levels
persistent_undo = false       # Save undo history to file
backup = false                # Create backup files
swap_file = false             # Create swap files
auto_save = false             # Automatically save files

[interface]
show_status_line = true       # Show status line
command_timeout = 1000        # Timeout for key sequences (ms)
show_command = true           # Show partial commands
scroll_off = 0                # Lines to keep above/below cursor
```

#### Keymap Configuration (`keymaps.toml`)

The `keymaps.toml` file defines mode-specific keybindings with complete customization:

```toml
[normal_mode]
"h" = "cursor_left"
"j" = "cursor_down"
"k" = "cursor_up"
"l" = "cursor_right"
"dd" = "delete_line"
"yy" = "yank_line"
"/" = "search_forward"
"n" = "find_next"
"N" = "find_previous"

[insert_mode]
"Escape" = "normal_mode"
"Char" = "insert_char"
"Backspace" = "delete_char_backward"

[command_mode]
"Enter" = "execute_command"
"Escape" = "normal_mode"
"Char" = "command_char"
```

#### Persistent Configuration

- **Automatic Saving**: All `:set` commands automatically save to `editor.toml`
- **Startup Loading**: Configuration loaded automatically when editor starts
- **UI Synchronization**: Settings changes immediately update the UI
- **Vim Compatibility**: Traditional Vim commands work with modern TOML backend

#### Available Configuration Settings

**Display Settings:**

- `show_line_numbers` - Show absolute line numbers (`:set number`)
- `show_relative_numbers` - Show relative line numbers (`:set relativenumber`)
- `show_cursor_line` - Highlight current cursor line (`:set cursorline`)
- `color_scheme` - Color scheme selection
- `syntax_highlighting` - Enable syntax highlighting

**Behavior Settings:**

- `tab_width` - Width of tab characters (`:set tabstop`)
- `expand_tabs` - Use spaces instead of tabs (`:set expandtab`)
- `auto_indent` - Automatic indentation
- `ignore_case` - Case-insensitive search (`:set ignorecase`)
- `smart_case` - Smart case sensitivity (`:set smartcase`)
- `highlight_search` - Highlight search results (`:set hlsearch`)
- `incremental_search` - Show matches while typing (`:set incsearch`)
- `wrap_lines` - Wrap long lines (`:set wrap`)

**Editing Settings:**

- `undo_levels` - Number of undo operations to remember
- `persistent_undo` - Save undo history between sessions
- `backup` - Create backup files
- `swap_file` - Create swap files
- `auto_save` - Automatically save files

**Interface Settings:**

- `show_status_line` - Display status line
- `command_timeout` - Timeout for key sequences (milliseconds)
- `show_command` - Show partial commands
- `scroll_off` - Lines to keep visible above/below cursor

## Architecture

### Core Modules

- **Editor**: Main editor state and coordination with TOML-based configuration
- **Config**: TOML configuration management for both keymaps and editor settings
- **Buffer**: Text buffer with undo/redo support and clipboard operations
- **Terminal**: Raw terminal interface using crossterm
- **Keymap**: TOML-based configurable key handling with sequence support
- **UI**: Rendering engine for status line, line numbers, cursor line, and content
- **Mode**: Editor mode definitions and cursor positioning
- **Search**: Regex-capable search engine with result navigation

### Planned Modules

- **Syntax**: Tree-sitter integration for syntax highlighting
- **LSP**: Language Server Protocol client
- **Plugin**: Lua scripting and plugin system

## Development Roadmap

### Phase 1: Core Vim Features ✅

- [x] **TOML-based Keymap System**: Configurable keybindings (revolutionary departure from traditional Vim)
- [x] **TOML-based Editor Configuration**: Comprehensive settings system with 30+ options
- [x] **Persistent Configuration**: All `:set` commands save to `editor.toml` automatically
- [x] **Search Engine**: Regex-capable search with `/`, `n`, `N` navigation
- [x] **Line Numbers**: Absolute (`:set number`), relative (`:set relativenumber`), and hybrid modes
- [x] **Cursor Line Highlighting**: Visual cursor line highlighting (`:set cursorline`)
- [x] **Key Sequences**: Multi-character commands like `dd`, `gg`, `yy` with timeout support
- [x] **Copy/Paste Operations**: `yy` (yank line), `yw` (yank word), `y$` (yank to end), `p/P` (put)
- [x] **Word Movement**: `w` (next word), `b` (previous word), `e` (word end)
- [x] **Delete Operations**: `x`, `X`, `dd` for character and line deletion
- [x] **Line Navigation**: `0` (line start), `$` (line end), `gg` (buffer start), `G` (buffer end)
- [ ] Text objects (`aw`, `iw`, `ap`, etc.)
- [ ] Operators (`d`, `c`, `y`, `p`) with motions
- [ ] Visual mode selection and operations
- [ ] Advanced search and replace with regex

### Phase 2: Advanced Editing 📅

- [ ] Macros and command repetition
- [ ] Code folding and auto-indentation  
- [ ] Multiple windows and tabs
- [ ] File explorer and buffer management

### Phase 3: IDE Features 📅

- [ ] LSP client with autocompletion
- [ ] Syntax highlighting with Tree-sitter
- [ ] Diagnostics and error handling
- [ ] Go-to definition and hover info

### Phase 4: Extensibility 📅

- [ ] Lua scripting API
- [ ] Plugin system and package manager
- [ ] Custom keybindings and commands
- [ ] Theme and color scheme support

## Dependencies

- **crossterm**: Cross-platform terminal manipulation
- **toml**: TOML parsing for configuration files (`keymaps.toml`, `editor.toml`)
- **serde**: Serialization framework for TOML configuration
- **anyhow**: Error handling for configuration and file operations
- **regex**: Pattern matching for search functionality
- **unicode-width/unicode-segmentation**: Proper Unicode text handling
- **tokio**: Async runtime for LSP and file operations (planned)
- **tree-sitter**: Syntax parsing and highlighting (planned)
- **notify**: File system watching (planned)
- **log/env_logger**: Logging infrastructure (planned)

## Contributing

This is a learning project to understand how text editors work. Contributions welcome!

### Getting Started

1. Clone the repository
2. Install Rust (1.70+)
3. Run `cargo run` to test the editor
4. Check issues for features to implement

### Code Structure

- Keep modules focused and well-documented
- Follow Rust best practices
- Add tests for new functionality
- Update this README with new features

## Inspiration

This project is inspired by:

- [Neovim](https://neovim.io/) - The extensible Vim editor
- [Helix](https://helix-editor.com/) - A Kakoune-inspired editor
- [Xi Editor](https://xi-editor.io/) - A modern text editor with async architecture

## License

MIT License - See LICENSE file for details
