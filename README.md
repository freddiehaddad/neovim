# Neovim Clone

A Vim/Neovim clone written in Rust with full feature parity as the goal.

## Current Features ✅

### Core Editor Functionality

- **Modal Editing**: Normal, Insert, Command, Visual, Replace, and Search modes
- **Buffer Management**: Multiple buffer support with undo/redo
- **Basic Text Operations**: Insert, delete, line breaks
- **Cursor Movement**: hjkl movement, arrow keys
- **Terminal Interface**: Raw terminal input/output with crossterm
- **Status Line**: Shows current mode, file info, cursor position

### Vim Keybindings

- **Insert Mode**: `i`, `I`, `a`, `A`, `o`, `O`
- **Navigation**: `h/j/k/l`, arrow keys
- **Mode Switching**: `Esc`, `:`, `/`, `v`, `V`
- **Undo/Redo**: `u`, `Ctrl+r`
- **File Operations**: `:w`, `:q`, `:wq`, `:e filename`

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

## Architecture

### Core Modules

- **Editor**: Main editor state and coordination
- **Buffer**: Text buffer with undo/redo support
- **Terminal**: Raw terminal interface using crossterm
- **Keymap**: Modal key handling and command execution
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

- [ ] Text objects (`aw`, `iw`, `ap`, etc.)
- [ ] Operators (`d`, `c`, `y`, `p`) with motions
- [ ] Word movement (`w`, `b`, `e`)
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
- **serde**: JSON serialization for LSP and config
- **tree-sitter**: Syntax parsing and highlighting
- **regex**: Pattern matching for search/replace
- **anyhow/thiserror**: Error handling

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
