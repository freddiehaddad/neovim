# Oxidized: A High-Performance Vim Clone in Rust

**Oxidized** is a modern terminal-based text editor that brings Vim's powerful modal editing to the 21st century. Built from the ground up in Rust, it combines Vim's time-tested editing philosophy with cutting-edge architecture, delivering exceptional performance, memory safety, and extensibility.

## 🚀 Key Features

### Revolutionary Configuration System

- **TOML-Based Configuration**: Replace Vim's cryptic rc files with intuitive, structured TOML configuration
- **Live Reloading**: Configuration changes apply instantly without restart
- **Automatic Persistence**: `:set` commands automatically save to configuration files

### Advanced Text Editing Engine  

- **Complete Modal System**: Normal, Insert, Command, Visual, Replace, and Search modes
- **Professional Text Objects**: Full support for words, paragraphs, quotes, brackets, tags, and custom objects
- **Operator Integration**: All operators (`d`, `c`, `y`, `>`, `<`, `~`) work seamlessly with text objects
- **Sophisticated Undo System**: Multi-level undo/redo with full operation tracking

### Modern Performance Architecture

- **Async Syntax Highlighting**: Background Tree-sitter processing with priority-based rendering
- **Multi-Buffer Management**: Efficient buffer handling with instant switching
- **Advanced Window System**: Complete window splitting, navigation, and resizing
- **Optimized Rendering**: Smart viewport management and efficient screen updates

### Cross-Platform Terminal Integration

- **Alternate Screen Mode**: Clean terminal entry/exit without disrupting scrollback
- **Unicode Support**: Full Unicode text handling with proper width calculation
- **Configurable Timeouts**: Customizable key sequence and mode transition timings

## 🔧 Installation & Setup

### Prerequisites

- **Rust 1.70+** - Install from [rustup.rs](https://rustup.rs/)
- **Terminal**: Modern terminal emulator with Unicode support

### Windows Installation

```powershell
# Install Rust (if not already installed)
Invoke-RestMethod -Uri https://win.rustup.rs/ | Invoke-Expression

# Clone and build Oxidized
git clone https://github.com/freddiehaddad/oxidized.git
cd oxidized
cargo build --release

# Run the editor
cargo run filename.txt

# Or use the built binary
.\target\release\oxy.exe filename.txt

# Install system-wide (optional)
cargo install --path .
# Binary will be available as 'oxy' in your PATH
```

### Linux/macOS Installation

```bash
# Install Rust (if not already installed)  
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Clone and build Oxidized
git clone https://github.com/freddiehaddad/oxidized.git
cd oxidized
cargo build --release

# Run the editor
cargo run filename.txt

# Or use the built binary
./target/release/oxy filename.txt

# Install system-wide (optional)
cargo install --path .
# Binary will be available as 'oxy' in your PATH
```

## 📖 Quick Start Guide

### First Steps

1. **Launch**: `oxy filename.txt` or `oxy` for a new buffer
2. **Insert Text**: Press `i` to enter Insert mode, type your content
3. **Navigate**: Use `hjkl` or arrow keys in Normal mode
4. **Save**: Type `:w` to write the file
5. **Exit**: Type `:q` to quit, or `:wq` to save and quit

### Essential Commands

**Basic Movement:**

- `hjkl` - Character movement (left/down/up/right)
- `w/b/e` - Word movement (next/previous/end)
- `0/$` - Line start/end
- `gg/G` - Buffer start/end

**Editing Operations:**

- `i/a` - Insert mode (before/after cursor)
- `dd` - Delete line
- `yy` - Copy line  
- `p/P` - Paste (after/before cursor)
- `x` - Delete character

**Search & Navigation:**

- `/pattern` - Search forward
- `?pattern` - Search backward
- `n/N` - Next/previous search result

### Advanced Window Management

**Window Creation:**

- `:split` or `:sp` - Horizontal split
- `:vsplit` or `:vsp` - Vertical split
- `Ctrl+w s/v` - Direct split creation

**Window Navigation:**

- `Ctrl+w hjkl` - Move between windows
- `Ctrl+w c` - Close current window
- `Ctrl+w o` - Close all other windows

**Window Resizing:**

- `Ctrl+w >/<` - Wider/narrower
- `Ctrl+w +/-` - Taller/shorter
- `Ctrl+w =` - Equalize sizes

## ⚙️ Configuration System

Oxidized uses a revolutionary TOML-based configuration system that's both human-readable and powerful.

### Editor Settings (`editor.toml`)

```toml
[display]
show_line_numbers = false
show_relative_numbers = true
show_cursor_line = true
color_scheme = "default"
syntax_highlighting = true

[behavior]
tab_width = 4
expand_tabs = false
auto_indent = true
ignore_case = false
smart_case = false
highlight_search = true
incremental_search = true
wrap_lines = false
line_break = false

[editing]
undo_levels = 1000
persistent_undo = false
backup = false
swap_file = false
auto_save = false
text_object_timeout = 1000
operator_pending_timeout = 1000

[interface]
show_status_line = true
status_line_format = "default"
command_timeout = 1000
show_command = true
scroll_off = 3
side_scroll_off = 0
window_resize_amount = 1
completion_menu_width = 30
completion_menu_height = 8

[languages]
default_language = "text"

[languages.extensions]
"rs" = "rust"
"toml" = "toml"
"md" = "markdown"
"txt" = "text"
"json" = "json"
```

### Keymap Customization (`keymaps.toml`)

```toml
[normal_mode]
# Movement
"h" = "cursor_left"
"j" = "cursor_down"
"k" = "cursor_up"
"l" = "cursor_right"
"w" = "word_forward"
"b" = "word_backward"
"e" = "word_end"
"0" = "line_start"
"^" = "line_first_char"
"$" = "line_end"
"gg" = "buffer_start"
"G" = "buffer_end"

# Mode transitions
"i" = "insert_mode"
"a" = "insert_after"
"o" = "insert_line_below"
"v" = "visual_mode"

# Delete/Edit operations
"x" = "delete_char_at_cursor"
"dd" = "delete_line"
"yy" = "yank_line"
"p" = "put_after"
"P" = "put_before"

# Search
"/" = "search_forward"
"n" = "search_next"
"N" = "search_previous"

# Window management
"Ctrl+w s" = "split_horizontal"
"Ctrl+w v" = "split_vertical"
"Ctrl+w h" = "window_left"
"Ctrl+w j" = "window_down"
"Ctrl+w k" = "window_up"
"Ctrl+w l" = "window_right"

# Viewport control
"zz" = "center_cursor"
"zt" = "cursor_to_top"
"zb" = "cursor_to_bottom"

# Scrolling
"Ctrl+f" = "scroll_down_page"
"Ctrl+b" = "scroll_up_page"
"Ctrl+d" = "scroll_down_half_page"
"Ctrl+u" = "scroll_up_half_page"
```

### Theme Configuration (`themes.toml`)

```toml
# Theme configuration for oxidized editor
[theme]
current = "default"

[themes.default]
name = "Rust Theme"
description = "Rust-inspired color palette with warm oranges and earth tones"

[themes.default.ui]
background = "#1f1611"
status_bg = "#ce422b"
status_fg = "#ffffff"
status_modified = "#f74c00"
line_number = "#8c6239"
line_number_current = "#deb887"
cursor_line_bg = "#2d2318"
empty_line = "#4a3728"
command_line_bg = "#1f1611"
command_line_fg = "#deb887"
selection_bg = "#8c4a2b"
warning = "#ff8c00"
error = "#dc322f"

[themes.default.tree_sitter]
# Rust-inspired color scheme with warm earth tones
keyword = "#ce422b"      # Rust orange for keywords
function = "#b58900"     # Golden brown for function names
type = "#268bd2"         # Steel blue for types
string = "#859900"       # Olive green for strings
number = "#d33682"       # Magenta for numbers
comment = "#93a1a1"      # Light gray for comments
identifier = "#deb887"   # Burlywood for identifiers
variable = "#deb887"     # Burlywood for variables
operator = "#cb4b16"     # Orange-red for operators
punctuation = "#839496"  # Gray for punctuation
```

## 🏗️ Architecture Overview

### Core Components

**Editor Engine:**

- **Modal System**: Complete implementation of Normal, Insert, Command, Visual, Replace, and Search modes
- **Buffer Management**: Multi-buffer support with efficient switching and state management
- **Window System**: Advanced window splitting, navigation, and resizing with independent viewports
- **Undo Engine**: Sophisticated multi-level undo/redo with operation tracking

**Rendering Pipeline:**

- **Async Syntax Highlighter**: Background Tree-sitter processing with priority queues
- **Viewport Manager**: Efficient screen updates with scroll optimization
- **Terminal Interface**: Cross-platform terminal handling with alternate screen support
- **Unicode Engine**: Proper Unicode text handling with width calculation

**Configuration Framework:**

- **TOML Parser**: Structured configuration with automatic validation
- **Hot Reloading**: Live configuration updates with file system watching
- **Command Integration**: `:set` commands persist automatically to TOML files
- **Theme Engine**: Dynamic theme switching with semantic color schemes

### Performance Features

- **Zero-Copy Rendering**: Efficient screen updates without unnecessary allocations
- **Background Processing**: Syntax highlighting and file operations run asynchronously
- **Memory Management**: Rust's ownership system ensures memory safety without garbage collection
- **Optimized Data Structures**: Efficient text manipulation with gap buffers and rope structures

## 📋 Feature Status

### ✅ Implemented Features

**Core Editing:**

- Complete modal editing system (Normal, Insert, Command, Visual, Replace, Search)
- Professional text objects (words, paragraphs, quotes, brackets, tags)
- Full operator support (`d`, `c`, `y`, `>`, `<`, `~`) with text object integration
- Multi-level undo/redo system with operation tracking
- Sophisticated clipboard operations with line/character modes

**Navigation & Movement:**

- Character movement (`hjkl`, arrow keys)
- Word movement (`w`, `b`, `e`, `W`, `B`, `E`)
- Line navigation (`0`, `^`, `$`, `gg`, `G`)
- Viewport control (`zz`, `zt`, `zb`)
- Scrolling commands (page, half-page, line scrolling)

**Window Management:**

- Complete window splitting system (horizontal/vertical)
- Window navigation with `Ctrl+w` combinations
- Dynamic window resizing with configurable increments
- Independent viewport and cursor positioning per window
- Visual window borders and active window indication

**Search & Replace:**

- Regex-capable search engine with forward/backward search
- Incremental search with live result highlighting
- Search result navigation (`n`, `N`)
- Case-sensitive and case-insensitive search modes

**Configuration System:**

- TOML-based configuration files (`editor.toml`, `keymaps.toml`, `themes.toml`)
- Live configuration reloading with file system watching
- Automatic persistence of `:set` commands
- Over 30 configurable editor settings

**Syntax Highlighting:**

- Async Tree-sitter integration with background processing
- Priority-based syntax highlighting for visible regions
- Rust language support with semantic color schemes
- Configurable themes with semantic color meaning

**Terminal Integration:**

- Alternate screen mode for clean entry/exit
- Cross-platform terminal handling (Windows, Linux, macOS)
- Unicode support with proper width calculation
- Professional status line with mode indication

### 🚧 In Progress

**Advanced Editing:**

- Visual mode selection and operations
- Advanced search and replace with regex substitution
- Code folding and automatic indentation
- Macro recording and playback

**File Management:**

- File explorer and directory navigation
- Advanced buffer management with session support
- File type detection and language-specific settings

### 📅 Planned Features

**IDE Integration:**

- LSP (Language Server Protocol) client integration
- Autocompletion with intelligent suggestions
- Go-to definition and hover information
- Diagnostics and error highlighting

**Extensibility:**

- Lua scripting API for custom commands and functions
- Plugin system with package management
- Custom syntax highlighting definitions
- User-defined text objects and operators

**Advanced Features:**

- Git integration with diff highlighting
- Terminal emulator within the editor
- Project-wide search and replace
- Session management with workspace support

## 🛠️ Development & Debugging

### Building from Source

**Development Build:**

```bash
# Clone repository
git clone https://github.com/freddiehaddad/oxidized.git
cd oxidized

# Build in debug mode (includes comprehensive logging)
cargo build

# Run with automatic trace logging
cargo run filename.txt
# Logs automatically written to oxidized.log
```

**Release Build:**

```bash
# Optimized release build
cargo build --release

# Run with custom log level
RUST_LOG=debug ./target/release/oxy filename.txt
```

### Cross-Platform Commands

**Windows (PowerShell):**

```powershell
# Build and run
cargo build
cargo run filename.txt

# Monitor logs in real-time
Get-Content oxidized.log -Wait

# Run tests
cargo test

# Install system-wide
cargo install --path .
```

**Linux/macOS (Bash):**

```bash
# Build and run
cargo build
cargo run filename.txt

# Monitor logs in real-time  
tail -f oxidized.log

# Run tests
cargo test

# Install system-wide
cargo install --path .
```

### Debugging and Logging

Oxidized includes comprehensive logging for development and debugging:

**Debug Builds:**

- Automatic trace-level logging to `oxidized.log`
- Detailed operation tracking and performance metrics
- Real-time log monitoring for development

**Release Builds:**

- Configurable logging via `RUST_LOG` environment variable
- Production-ready error handling and reporting
- Optional debug logging for troubleshooting

### Testing

```bash
# Run all tests (108+ tests across all components)
cargo test

# Run specific test suite
cargo test buffer_integration
cargo test search_integration
cargo test syntax_integration

# Run with verbose output
cargo test -- --nocapture
```

The test suite includes comprehensive integration tests covering:

- Buffer operations and text manipulation
- Search engine functionality with regex support
- Syntax highlighting with Tree-sitter integration
- Editor modes and state transitions
- Text objects and operator combinations
- Theme system and configuration management

## 🧰 Dependencies

### Core Dependencies

- **crossterm**: Cross-platform terminal manipulation and event handling
- **toml**: TOML configuration file parsing and serialization
- **serde**: Serialization framework for configuration management
- **anyhow**: Ergonomic error handling for operations
- **regex**: Regular expression engine for search functionality

### Advanced Features

- **tree-sitter**: Abstract syntax tree parsing for syntax highlighting
- **tree-sitter-rust**: Rust language grammar for Tree-sitter
- **notify**: File system monitoring for configuration hot reloading
- **tokio**: Async runtime for background processing
- **unicode-width/unicode-segmentation**: Unicode text handling

### Development

- **log/env_logger**: Logging infrastructure for debugging
- **criterion**: Benchmarking framework for performance testing

## 🤝 Contributing

Oxidized is an open-source learning project focused on understanding text editor architecture. Contributions are welcome!

### Getting Started

1. **Fork and Clone**: Fork the repository and clone your fork
2. **Set up Environment**: Install Rust 1.70+ and your preferred IDE
3. **Build and Test**: Run `cargo build && cargo test` to ensure everything works
4. **Pick an Issue**: Check the issue tracker for features to implement
5. **Submit PR**: Create a pull request with your changes

### Development Guidelines

- **Code Quality**: Follow Rust best practices and use `rustfmt` for formatting
- **Testing**: Add tests for new functionality and ensure existing tests pass
- **Documentation**: Update documentation and comments for new features
- **Architecture**: Maintain clean module separation and well-defined interfaces

### Areas for Contribution

- **Feature Implementation**: Help implement planned features from the roadmap
- **Performance Optimization**: Improve rendering speed and memory usage
- **Platform Support**: Enhance cross-platform compatibility
- **Documentation**: Improve user guides and developer documentation
- **Testing**: Add more comprehensive test coverage

---

## 🎯 Vim/Neovim Feature Parity Roadmap

This section outlines our plan to achieve complete feature parity with Vim/Neovim while maintaining oxidized's performance advantages.

### ✅ **Currently Implemented**

- **Modal Editing**: Complete with Normal, Insert, Visual, Command, Replace, Search modes
- **Basic Movement**: hjkl, word movement (w/b/e), line navigation (0/$, gg/G)
- **Text Objects**: Comprehensive implementation with words, paragraphs, quotes, brackets
- **Operators**: Full operator system (d/c/y/>/</~) with text object integration
- **Window Management**: Splits, navigation, resizing with Ctrl+w commands  
- **Buffer Management**: Multi-buffer support with switching and management
- **Search**: Forward/backward search with n/N navigation
- **Undo/Redo**: Multi-level undo system with operation tracking
- **Configuration**: TOML-based config with live reloading
- **Syntax Highlighting**: Tree-sitter integration with async processing
- **Clipboard Operations**: Basic yank/put with character and line modes
- **Scrolling**: Complete scrolling system (Ctrl+f/b/d/u, zz/zt/zb)
- **Command System**: Ex-commands with completion (:w, :q, :set, etc.)
- **Cursor Shape**: Mode-aware cursor changes (block/line/underline)

### 🚧 **Phase 1: Essential Vim Features (High Priority)**

#### 1. **Macro System**

```rust
// Priority: CRITICAL - Core Vim feature
// Implementation: src/features/macros.rs
pub struct MacroRecorder {
    recording_register: Option<char>,
    current_macro: Vec<KeyEvent>,
    stored_macros: HashMap<char, Vec<KeyEvent>>,
}
```

- **q{register}**: Start/stop macro recording
- **@{register}**: Playback macro
- **@@**: Repeat last macro
- **{count}@{register}**: Repeat macro count times

#### 2. **Named Registers System**

```rust
// Priority: HIGH - Essential for advanced editing
// Implementation: src/features/registers.rs
pub struct RegisterSystem {
    named_registers: HashMap<char, ClipboardContent>,    // a-z
    numbered_registers: VecDeque<ClipboardContent>,      // 0-9
    special_registers: HashMap<char, ClipboardContent>,  // "/%, etc.
}
```

- **"{register}**: Access named registers (a-z, A-Z)
- **Numbered registers**: 0-9 for deleted text
- **Special registers**: "/, "%, ":, "., etc.

#### 3. **Complete Visual Mode Operations**

- **Visual selection**: Proper character selection with highlighting
- **Visual line selection**: Complete line selection (V)
- **Visual block selection**: Rectangular selection (Ctrl+V)
- **Selection operations**: d, c, y, >, <, ~ with visual selections

#### 4. **Enhanced Search & Replace**

- **Search history**: Up/Down arrows in search mode
- **Search options**: \c, \C for case sensitivity
- **Substitute command**: :s/pattern/replacement/flags
- **Global replace**: :%s/pattern/replacement/g
- **Interactive replace**: Confirmation prompts

#### 5. **Marks and Jumps**

```rust
// Priority: MEDIUM-HIGH - Navigation enhancement
// Implementation: src/features/marks.rs
pub struct MarkSystem {
    local_marks: HashMap<char, Position>,     // a-z
    global_marks: HashMap<char, (PathBuf, Position)>, // A-Z
    jump_list: VecDeque<Position>,
}
```

- **m{mark}**: Set local marks (a-z) and global marks (A-Z)
- **'{mark}**: Jump to mark
- **Ctrl+O/Ctrl+I**: Navigate jump list

### 🔧 **Phase 2: Advanced Vim Features (Medium Priority)**

#### 6. **Tabs Support**

- **:tabnew**: Create new tab
- **gt/gT**: Navigate tabs
- **:tabclose**: Close current tab

#### 7. **Complete Character Navigation**

- **f/F/t/T**: Enhanced character finding with repeat
- **;/,**: Repeat character search forward/backward
- **Bracket matching**: % for bracket/quote/tag matching

#### 8. **Improved Undo System**

- **Undo branches**: g+/g- for undo tree navigation
- **:undolist**: Show undo history
- **Earlier/later**: :earlier 5m, :later 10s

#### 9. **Ex Command System Enhancement**

- **More ex-commands**: :copy, :move, :delete, :join
- **Command ranges**: :1,5d, :.,+3y, :%s//
- **Command history**: Up/Down arrows in command mode

#### 10. **Folding System**

- **zf**: Create fold
- **zo/zc**: Open/close fold
- **zM/zR**: Close/open all folds
- **Fold methods**: Manual, indent-based, syntax-based

### 🌟 **Phase 3: Modern Features (Medium-Low Priority)**

#### 11. **Complete LSP Integration**

- **Go to definition**: gd
- **Find references**: gr
- **Hover information**: K
- **Diagnostics**: Real-time error highlighting
- **Code actions**: Refactoring suggestions

#### 12. **Plugin System**

- **Lua scripting**: Full mlua integration
- **Plugin manager**: Install/update plugins
- **API bindings**: Expose editor functionality to Lua

#### 13. **Git Integration**

- **Git status**: Show modified lines in gutter
- **Git blame**: :Gblame command
- **Git diff**: :Gdiff command

### 📈 **Implementation Strategy**

**Phase 1 Timeline** (16-20 weeks):

1. **Macro System**: 4-6 weeks
2. **Named Registers**: 2-3 weeks  
3. **Visual Mode Completion**: 3-4 weeks
4. **Search & Replace**: 3-4 weeks
5. **Marks & Jumps**: 2-3 weeks

**Success Metrics**:

- **90% Vim compatibility**: Most common Vim workflows work identically
- **Performance**: Sub-100ms response time for all operations
- **Stability**: No crashes during normal editing sessions

**Contributing**: Pick any feature from Phase 1 to start contributing! Each feature is designed to be implemented independently.

## 💡 Inspiration

Oxidized draws inspiration from exceptional editors that have shaped the text editing landscape:

- **[Vim](https://www.vim.org/)**: The legendary modal editor that defined efficient text manipulation
- **[Neovim](https://neovim.io/)**: The extensible, modernized Vim with Lua scripting
- **[Helix](https://helix-editor.com/)**: A Kakoune-inspired editor with Tree-sitter integration
- **[Xi Editor](https://xi-editor.io/)**: Google's experimental editor with async architecture

## 📜 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

---

**Ready to experience the future of text editing?** Install Oxidized today and discover what happens when Vim's power meets Rust's performance and safety!
