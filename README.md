# Neovim Clone

A Vim/Neovim clone written in Rust with full feature parity as the goal.

## Current Features ✅

### Core Editor Functionality

- **Modal Editing**: Normal, Insert, Command, Visual, Replace, and Search modes
- **TOML-based Keymap System**: Configurable keybindings loaded from `keymaps.toml`
- **Key Sequence Support**: Multi-character commands like `dd`, `gg`, `yy` with timeout
- **Buffer Management**: Multiple buffer support with undo/redo
- **Copy/Paste System**: Vim-compatible yank/put operations with type awareness
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

## Usage

```bash
# Build the project
cargo build

# Run without opening a file
cargo run

# Open a specific file
cargo run -- filename.txt
```

### Basic Usage

1. Start in Normal mode
2. Press `i` to enter Insert mode and type text
3. Press `Esc` to return to Normal mode
4. Use `hjkl` or arrow keys to move cursor
5. Type `:w` to save, `:q` to quit

### Configuration

The editor uses a revolutionary TOML-based keymap system that breaks from traditional Vim by making all keybindings configurable. The `keymaps.toml` file defines mode-specific keybindings:

```toml
[normal_mode]
"h" = "cursor_left"
"j" = "cursor_down"
"dd" = "delete_line"
"yy" = "yank_line"

[insert_mode]
"Escape" = "normal_mode"
"Char" = "insert_char"
```

This allows complete customization of the editor's behavior while maintaining Vim compatibility by default.

## Architecture

### Core Modules

- **Editor**: Main editor state and coordination
- **Buffer**: Text buffer with undo/redo support and clipboard operations
- **Terminal**: Raw terminal interface using crossterm
- **Keymap**: TOML-based configurable key handling with sequence support
- **UI**: Rendering engine for status line and content
- **Mode**: Editor mode definitions and cursor positioning

### Planned Modules

- **Syntax**: Tree-sitter integration for syntax highlighting
- **LSP**: Language Server Protocol client
- **Search**: Pattern matching and text search
- **Config**: Configuration file parsing
- **Plugin**: Lua scripting and plugin system

## Development Roadmap

### Phase 1: Core Vim Features ⏳

- [x] **TOML-based Keymap System**: Configurable keybindings (revolutionary departure from traditional Vim)
- [x] **Key Sequences**: Multi-character commands like `dd`, `gg`, `yy` with timeout support
- [x] **Copy/Paste Operations**: `yy` (yank line), `yw` (yank word), `y$` (yank to end), `p/P` (put)
- [x] **Word Movement**: `w` (next word), `b` (previous word), `e` (word end)
- [x] **Delete Operations**: `x`, `X`, `dd` for character and line deletion
- [x] **Line Navigation**: `0` (line start), `$` (line end), `gg` (buffer start), `G` (buffer end)
- [ ] Text objects (`aw`, `iw`, `ap`, etc.)
- [ ] Operators (`d`, `c`, `y`, `p`) with motions
- [ ] Visual mode selection and operations
- [ ] Search and replace with regex

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
- **tokio**: Async runtime for LSP and file operations
- **serde/serde_json**: JSON serialization for LSP and data structures
- **toml**: TOML parsing for configuration files (keymaps.toml)
- **regex**: Pattern matching for search/replace
- **anyhow/thiserror**: Error handling
- **unicode-width/unicode-segmentation**: Proper Unicode text handling
- **notify**: File system watching
- **tree-sitter**: Syntax parsing and highlighting (planned)
- **log/env_logger**: Logging infrastructure

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
