# Neovim Clone

A revolutionary Vim/Neovim clone written in Rust that breaks from traditional Vim with a **TOML-first configuration system** for both keymaps and editor settings.

## 🚀 Revolutionary Features

### ⚡ TOML-Based Configuration System

Unlike traditional Vim, this editor uses **human-readable TOML files** for all configuration:

- **`keymaps.toml`** - Complete keymap customization
- **`editor.toml`** - Comprehensive editor settings (30+ options)  
- **`themes.toml`** - Advanced syntax highlighting themes and UI colors
- **No more scattered hardcoded defaults** - everything is configurable
- **Persistent settings** - all `:set` commands automatically save to configuration

### 🎨 Tree-sitter Syntax Highlighting

- **Professional-grade syntax highlighting** powered by Tree-sitter AST parsing
- **Rust-inspired color schemes** with semantic meaning and visual identity
- **Multi-language support** with extensible language definitions
- **Real-time highlighting** with accurate syntax recognition
- **Customizable themes** including default, dark, light, and special "Ferris" theme

### 🎯 Familiar Vim Interface with Modern Backend

- Full Vim command compatibility (`:set number`, `:set relativenumber`, etc.)
- TOML configuration with Vim-style aliases (`number`/`nu`, `relativenumber`/`rnu`)
- Automatic configuration loading and UI synchronization

## Current Features ✅

### Core Editor Functionality

- **Modal Editing**: Normal, Insert, Command, Visual, Replace, and Search modes
- **TOML-based Keymap System**: Configurable keybindings loaded from `keymaps.toml`
- **TOML-based Editor Settings**: Comprehensive configuration in `editor.toml`
- **Tree-sitter Syntax Highlighting**: Professional AST-based syntax highlighting with built-in Rust support
- **Multi-Buffer Support**: Complete buffer management with Ex commands (`:e`, `:b`, `:bd`, `:ls`)
- **Window Management**: Full window splitting and navigation system with Vim-style keybindings
- **Rust-Inspired Color Schemes**: Beautiful themes reflecting Rust's safety and performance values
- **Key Sequence Support**: Multi-character commands like `dd`, `gg`, `yy` with timeout
- **Advanced Search Engine**: Regex-capable search with `/`, `n`, `N` navigation
- **Line Numbers**: Absolute, relative, and hybrid line number modes
- **Cursor Line Highlighting**: Visual cursor line highlighting
- **Configuration Hot Reloading**: Live updates when TOML files change
- **Professional Terminal Behavior**: Alternate screen support for clean entry/exit
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
- **Window Management**:
  - `Ctrl+w s` - Horizontal split
  - `Ctrl+w v` - Vertical split
  - `Ctrl+w h/j/k/l` - Navigate between windows
  - `Ctrl+w c` - Close current window

### Ex Commands

- `:q` - Quit
- `:q!` - Force quit
- `:w` - Save file
- `:wq` - Save and quit
- `:e <filename>` - Open file in new buffer
- `:b <buffer>` - Switch to buffer by number or name
- `:bd` - Delete current buffer
- `:ls` - List all open buffers

### Window Management Commands

- `:split` / `:sp` - Create horizontal split
- `:vsplit` / `:vsp` - Create vertical split
- Window navigation with `Ctrl+w` + direction (`h`, `j`, `k`, `l`)
- Each window maintains independent viewport and can display different buffers

### Search Commands

- `/pattern` - Search forward for pattern (supports regex)
- `n` - Go to next search result
- `N` - Go to previous search result

### Configuration Commands

The editor now supports **all major Vim `:set` commands** with automatic persistence to `editor.toml`:

#### Display Settings

- `:set number` / `:set nonumber` (`nu` / `nonu`) - Toggle line numbers
- `:set relativenumber` / `:set norelativenumber` (`rnu` / `nornu`) - Toggle relative line numbers  
- `:set cursorline` / `:set nocursorline` (`cul` / `nocul`) - Toggle cursor line highlighting
- `:set syntax` / `:set nosyntax` (`syn` / `nosyn`) - Toggle syntax highlighting
- `:set colorscheme=<theme>` (`colo`) - Change color theme (default, dark, light, ferris)

#### Search & Navigation

- `:set ignorecase` / `:set noignorecase` (`ic` / `noic`) - Case-insensitive search
- `:set smartcase` / `:set nosmartcase` (`scs` / `noscs`) - Smart case matching
- `:set hlsearch` / `:set nohlsearch` (`hls` / `nohls`) - Highlight search results
- `:set incsearch` / `:set noincsearch` (`is` / `nois`) - Incremental search
- `:set scrolloff=<n>` (`so`) - Lines to keep around cursor
- `:set sidescrolloff=<n>` (`siso`) - Columns to keep around cursor

#### Text Editing

- `:set tabstop=<n>` (`ts`) - Tab width (default: 4)
- `:set expandtab` / `:set noexpandtab` (`et` / `noet`) - Use spaces instead of tabs
- `:set autoindent` / `:set noautoindent` (`ai` / `noai`) - Automatic indentation
- `:set wrap` / `:set nowrap` - Line wrapping
- `:set linebreak` / `:set nolinebreak` (`lbr` / `nolbr`) - Word boundary wrapping

#### File Management

- `:set backup` / `:set nobackup` (`bk` / `nobk`) - Create backup files
- `:set swapfile` / `:set noswapfile` (`swf` / `noswf`) - Enable swap files
- `:set autosave` / `:set noautosave` (`aw` / `noaw`) - Automatic file saving
- `:set undolevels=<n>` (`ul`) - Number of undo levels (default: 1000)
- `:set undofile` / `:set noundofile` (`udf` / `noudf`) - Persistent undo history

#### Interface

- `:set laststatus` / `:set nolaststatus` (`ls` / `nols`) - Show status line
- `:set showcmd` / `:set noshowcmd` (`sc` / `nosc`) - Show partial commands
- `:set timeoutlen=<ms>` (`tm`) - Command timeout (default: 1000ms)

#### Setting Queries

- `:set` - Show current basic settings
- `:set all` - Show all settings with values  
- `:set <option>?` - Query specific setting value

**All settings automatically persist to `editor.toml` and apply immediately!**

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
7. **Split windows**: Use `:vsplit` for vertical split, `:split` for horizontal split
8. **Navigate windows**: Use `Ctrl+w` + `h/j/k/l` to move between windows
9. **Save and quit**: Type `:wq`

### Window Management Guide

The editor features a complete window management system similar to Vim:

**Creating Splits:**

- `:split` or `:sp` - Create horizontal split (window above/below)
- `:vsplit` or `:vsp` - Create vertical split (window left/right)
- `Ctrl+w s` - Horizontal split keybinding
- `Ctrl+w v` - Vertical split keybinding

**Navigation:**

- `Ctrl+w h` - Move to window on the left
- `Ctrl+w j` - Move to window below
- `Ctrl+w k` - Move to window above  
- `Ctrl+w l` - Move to window on the right

**Features:**

- Each window maintains independent viewport and cursor position
- Windows can display different buffers or the same buffer
- Visual borders distinguish between windows
- Active window is clearly indicated

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

The editor features a **revolutionary TOML-based configuration system** that completely replaces traditional Vim's scattered settings approach, plus advanced Tree-sitter syntax highlighting.

#### Syntax Highlighting

Syntax highlighting is built-in and powered by Tree-sitter with automatic Rust language support. Colors are configured through the unified `themes.toml` file:

```toml
[themes.default.syntax]
# Rust-inspired syntax colors with semantic meaning
text = "#dbd7ca"              # Light cream - default text
comment = "#5c6773"           # Muted gray - comments
keyword = "#ce422b"           # Rust red/orange - core keywords
operator = "#ff6a00"          # Bright rust orange - operators  
type = "#86b300"              # Fresh green - types (safety)
struct = "#86b300"            # Fresh green - struct definitions
enum = "#86b300"              # Fresh green - enum definitions
string = "#b4a72e"            # Golden yellow - string literals
number = "#ff9940"            # Bright orange - numeric literals
boolean = "#ce422b"           # Rust red - boolean values
character = "#b4a72e"         # Golden yellow - character literals
function = "#39adb5"          # Teal blue - functions (reliability)
method = "#39adb5"            # Teal blue - method calls
macro = "#f29718"             # Vibrant orange - macros
variable = "#dbd7ca"          # Light cream - variables
parameter = "#dbd7ca"         # Light cream - parameters
property = "#59c2ff"          # Sky blue - properties/fields
constant = "#f29718"          # Vibrant orange - constants

# Alternative themes available: dark, light, ferris
```

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

# Window management keybindings
"Ctrl+w s" = "split_horizontal"
"Ctrl+w v" = "split_vertical"
"Ctrl+w h" = "move_to_window_left"
"Ctrl+w j" = "move_to_window_down"
"Ctrl+w k" = "move_to_window_up"
"Ctrl+w l" = "move_to_window_right"
"Ctrl+w c" = "close_window"

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

**Syntax Highlighting:**

- `enabled` - Enable/disable syntax highlighting globally
- `default_theme` - Active color theme ("default", "dark", "light", "ferris")
- **Language Support**: Extensible language definitions with Tree-sitter grammars
- **Color Themes**: Rust-inspired themes with semantic color meaning
- **Real-time Updates**: Live theme switching and configuration reloading

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
- **Config**: TOML configuration management for keymaps, editor settings, and syntax themes
- **Buffer**: Multi-buffer management with undo/redo support and clipboard operations
- **Window**: Advanced window management system with splitting and navigation
- **Terminal**: Raw terminal interface using crossterm with alternate screen support
- **Keymap**: TOML-based configurable key handling with sequence support
- **UI**: Advanced rendering engine for status line, line numbers, cursor line, multi-window support, and syntax-highlighted content
- **Mode**: Editor mode definitions and cursor positioning
- **Search**: Regex-capable search engine with result navigation
- **Syntax**: Tree-sitter integration for professional syntax highlighting with configurable themes

### Implemented Features ✅

- **Tree-sitter Syntax Highlighting**: AST-based syntax highlighting with Rust-inspired themes
- **Multi-Buffer System**: Complete buffer management with Ex commands
- **Window Management System**: Full window splitting and navigation with Vim-style keybindings
- **Configuration Hot Reloading**: Live updates when TOML configuration files change
- **Professional Terminal Behavior**: Alternate screen support for clean entry/exit

## Development Roadmap

### Phase 1: Core Vim Features ✅

- [x] **TOML-based Keymap System**: Configurable keybindings (revolutionary departure from traditional Vim)
- [x] **TOML-based Editor Configuration**: Comprehensive settings system with 30+ options
- [x] **Tree-sitter Syntax Highlighting**: Professional AST-based highlighting with Rust-inspired themes
- [x] **Multi-Buffer Support**: Complete buffer management with Ex commands (`:e`, `:b`, `:bd`, `:ls`)
- [x] **Window Management**: Full window splitting and navigation with Vim-style keybindings
- [x] **Configuration Hot Reloading**: Live updates when TOML files change
- [x] **Professional Terminal Behavior**: Alternate screen support for clean entry/exit
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
- [x] Multiple windows and tabs (✅ Window splitting implemented)
- [ ] File explorer and buffer management

### Phase 3: IDE Features 📅

- [ ] LSP client with autocompletion
- [x] Syntax highlighting with Tree-sitter (✅ Implemented)
- [ ] Diagnostics and error handling
- [ ] Go-to definition and hover info

### Phase 4: Extensibility 📅

- [ ] Lua scripting API
- [ ] Plugin system and package manager
- [ ] Custom keybindings and commands
- [ ] Theme and color scheme support

## Dependencies

- **crossterm**: Cross-platform terminal manipulation with alternate screen support
- **toml**: TOML parsing for configuration files (`keymaps.toml`, `editor.toml`, `themes.toml`)
- **serde**: Serialization framework for TOML configuration
- **anyhow**: Error handling for configuration and file operations
- **regex**: Pattern matching for search functionality
- **unicode-width/unicode-segmentation**: Proper Unicode text handling
- **tree-sitter**: AST-based syntax parsing and highlighting (✅ Implemented)
- **tree-sitter-rust**: Rust grammar for Tree-sitter (✅ Implemented)
- **notify**: File system watching for configuration hot reloading (planned)
- **tokio**: Async runtime for LSP and file operations (planned)
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
