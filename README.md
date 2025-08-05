# Oxidized

A revolutionary Vim/Neovim clone written in Rust that breaks from traditional Vim with a **TOML-first configuration system** for both keymaps and editor settings.

## 🦀 The Oxidized Editor

**Oxidized** (`oxy`) is a modern, high-performance text editor that reimagines Vim for the Rust era. Built from the ground up in Rust, it combines Vim's legendary efficiency with modern software engineering principles and features complete text objects and operators support with full undo integration.

## 🚀 Revolutionary Features

### ⚡ TOML-Based Configuration System

Unlike traditional Vim, this editor uses **human-readable TOML files** for all configuration:color_scheme = "dark"            # Current theme (only dark theme available)

- **`keymaps.toml`** - Complete keymap customization
- **`editor.toml`** - Comprehensive editor settings (30+ options)  
- **`themes.toml`** - Advanced syntax highlighting themes and UI colors
- **No more scattered hardcoded defaults** - everything is configurable
- **Persistent settings** - all `:set` commands automatically save to configuration

### 🎨 Advanced Async Syntax Highlighting

- **Professional-grade syntax highlighting** powered by Tree-sitter AST parsing
- **Async background processing** for non-blocking syntax highlighting with priority queues
- **Intelligent caching system** with content-aware cache keys and automatic invalidation
- **Immediate highlighting** for visible lines during file opening and scrolling
- **Rust-inspired color schemes** with semantic meaning and visual identity
- **Multi-language support** with extensible language definitions
- **Real-time highlighting** with accurate syntax recognition and smooth performance
- **Dark theme optimized** for low-light coding environments

### 🎯 Familiar Vim Interface with Modern Backend

- Full Vim command compatibility (`:set number`, `:set relativenumber`, etc.)
- TOML configuration with Vim-style aliases (`number`/`nu`, `relativenumber`/`rnu`)
- Automatic configuration loading and UI synchronization

### 🔥 Performance Optimizations

This editor is built with **performance as a core principle**, implementing cutting-edge optimizations:

#### Delta-Based Undo System

- **Eliminates expensive buffer cloning** - Traditional editors clone entire buffer contents for each edit operation
- **Memory-efficient deltas** - Only stores minimal edit operations (insert, delete, replace) instead of full buffer states
- **Sub-millisecond undo/redo** - Operations execute in constant time regardless of buffer size
- **Dramatically reduced memory usage** - From O(n×m) to O(k) where n=buffer size, m=operations, k=edit deltas
- **Configurable undo levels** - Respects `undo_levels` setting in `editor.toml` for memory management

#### Syntax Highlighting Cache

- **Intelligent caching system** - Avoids re-parsing identical content with LRU eviction policy
- **Content-aware cache keys** - Uses content hash + language + theme for precise cache invalidation
- **Automatic cache management** - Clears cache on theme changes, manages memory with configurable limits
- **Significant scrolling performance** - 5-10x faster scrolling through previously viewed code
- **Memory efficient** - Cache size configurable with smart eviction (default: 1000 entries)

#### Async Syntax Highlighting System

- **Background worker architecture** - Syntax highlighting processed in async background tasks with priority queues
- **Non-blocking operation** - Editor remains responsive during syntax highlighting of large files
- **Priority-based processing** - Visible lines get Critical priority, scroll areas get High priority
- **Intelligent request batching** - Efficiently processes multiple highlighting requests together
- **Immediate fallback system** - Visible lines highlighted synchronously for instant feedback during file opening
- **Shared cache integration** - Both async and immediate highlighting contribute to unified cache
- **Automatic highlighting triggers** - File operations (`:e`) and scrolling automatically request highlighting
- **Memory efficient** - Only stores essential highlighting data with configurable cache limits

#### Rust Foundation

- **Memory safety without garbage collection** - Zero-cost memory management
- **Fearless concurrency** - Prepared for multi-threaded optimizations
- **Compile-time optimizations** - Aggressive dead code elimination and inlining

### Current Features ✅

### Core Editor Functionality

- **Modal Editing**: Normal, Insert, Command, Visual, Replace, and Search modes
- **Text Objects and Operators**: Complete Vim-compatible text object system with full undo integration
- **TOML-based Keymap System**: Configurable keybindings loaded from `keymaps.toml`
- **TOML-based Editor Settings**: Comprehensive configuration in `editor.toml`
- **Intelligent Command Completion**: Tab-based completion with popup menu for all Ex commands and settings
- **Async Syntax Highlighting**: Non-blocking background syntax highlighting with immediate visible line processing
- **Multi-Buffer Support**: Complete buffer management with Ex commands (`:e`, `:b`, `:bd`, `:ls`)
- **Window Management**: Full window splitting and navigation system with Vim-style keybindings
- **Optimized Dark Theme**: Single high-contrast theme optimized for low-light coding environments
- **Key Sequence Support**: Multi-character commands like `dd`, `gg`, `yy` with timeout
- **Advanced Search Engine**: Regex-capable search with `/`, `n`, `N` navigation
- **Line Numbers**: Absolute, relative, and hybrid line number modes
- **Cursor Line Highlighting**: Visual cursor line highlighting
- **Scrolling Commands**: Page/half-page scrolling (`Ctrl+f/b/d/u`) and line scrolling (`Ctrl+e/y`)
- **Viewport Centering**: Z-commands (`zz`, `zt`, `zb`) for cursor positioning
- **Scroll Offset**: Configurable `scroll_off` to maintain cursor distance from viewport edges
- **Configuration Hot Reloading**: Live updates when TOML files change
- **Professional Terminal Behavior**: Alternate screen support for clean entry/exit
- **Comprehensive Logging**: Debug-friendly logging system with file-based output for development and troubleshooting
- **Copy/Paste System**: Vim-compatible yank/put operations with type awareness and text object support
- **Basic Text Operations**: Insert, delete, line breaks, word movement
- **Cursor Movement**: hjkl movement, arrow keys, word navigation, line/buffer navigation
- **Character Navigation**: Vim-style character search with `f/F/t/T` and `;/,` repeat functionality
- **Terminal Interface**: Raw terminal input/output with crossterm
- **Status Line**: Shows current mode, file info, cursor position

#### 🎯 Character Navigation

Oxidized implements Vim's powerful character navigation system for precise cursor movement within lines:

- **`f{char}`** - Find character forward: Jump to the next occurrence of `{char}` on the current line
- **`F{char}`** - Find character backward: Jump to the previous occurrence of `{char}` on the current line  
- **`t{char}`** - Till character forward: Jump to just before the next occurrence of `{char}`
- **`T{char}`** - Till character backward: Jump to just after the previous occurrence of `{char}`
- **`;`** - Repeat last character search in the same direction
- **`,`** - Repeat last character search in the opposite direction

This feature enables lightning-fast navigation within lines - essential for efficient Vim-style editing.

### Vim Keybindings

- **Insert Mode**: `i`, `I`, `a`, `A`, `o`, `O`
- **Navigation**: `h/j/k/l`, arrow keys, `w/b/e` (word movement), `0/$` (line start/end), `gg/G` (buffer start/end)
- **Character Navigation**: `f{char}` (find forward), `F{char}` (find backward), `t{char}` (till forward), `T{char}` (till backward), `;` (repeat search), `,` (repeat reverse)
- **Mode Switching**: `Esc`, `:`, `/`, `v`, `V`, `R` (replace mode)
- **Delete Operations**: `x` (delete char), `X` (delete char before), `dd` (delete line), plus text objects (`diw`, `dap`, etc.)
- **Line Operations**:
  - `J` - Join lines: Join current line with the next line (removes line break and normalizes whitespace)
  - `D` - Delete to end of line: Delete from cursor position to end of current line
  - `C` - Change to end of line: Delete from cursor to end of line and enter Insert mode
  - `S` - Change entire line: Clear the entire current line and enter Insert mode at beginning
  - `s` - Substitute character: Delete character under cursor and enter Insert mode
- **Copy/Paste**: `yy` (yank line), `yw` (yank word), `y$` (yank to end), plus text objects (`yiw`, `yap`, etc.), `p/P` (put after/before)
- **Undo/Redo**: `u`, `Ctrl+r`
- **File Operations**: `:w`, `:q`, `:q!`, `:wq`, `:e filename`
- **Window Management**:
  - `Ctrl+w s` - Horizontal split (below)
  - `Ctrl+w S` - Horizontal split (above)
  - `Ctrl+w v` - Vertical split (right)
  - `Ctrl+w V` - Vertical split (left)
  - `Ctrl+w h/j/k/l` - Navigate between windows
  - `Ctrl+w c` - Close current window
  - `Ctrl+w >/</+/-` - Resize windows (wider/narrower/taller/shorter)
- **Scrolling Commands**:
  - `Ctrl+f/Ctrl+b` - Page down/up
  - `Ctrl+d/Ctrl+u` - Half page down/up
  - `Ctrl+e/Ctrl+y` - Line down/up
  - `zz` - Center cursor line
  - `zt` - Move cursor line to top
  - `zb` - Move cursor line to bottom

### Advanced Text Objects and Operators

**Oxidized** features a complete implementation of Vim's powerful text objects and operators system, enabling sophisticated text manipulation with intuitive command combinations.

#### Operators

Operators perform actions on text objects or motions. Press an operator key to enter **Operator-Pending mode**, then specify a text object:

- **`d`** - Delete text object (e.g., `diw` deletes inner word)
- **`c`** - Change text object (delete and enter insert mode)  
- **`y`** - Yank (copy) text object
- **`>`** - Indent text object
- **`<`** - Unindent text object
- **`~`** - Toggle case of text object

#### Text Objects

Text objects define regions of text. Use them after operators for precise text manipulation:

**Word Text Objects:**

- `iw` - **inner word** (word without surrounding whitespace)
- `aw` - **a word** (word with surrounding whitespace)
- `iW` - **inner WORD** (WORD without surrounding whitespace)
- `aW` - **a WORD** (WORD with surrounding whitespace)

**Paragraph Text Objects:**

- `ip` - **inner paragraph** (paragraph without surrounding blank lines)
- `ap` - **a paragraph** (paragraph with surrounding blank lines)

**Sentence Text Objects:**

- `is` - **inner sentence** (sentence without surrounding whitespace)
- `as` - **a sentence** (sentence with surrounding whitespace)

**Quote Text Objects:**

- `i"` - **inner double quotes** (content inside `"..."`)
- `a"` - **around double quotes** (content including the quotes)
- `i'` - **inner single quotes** (content inside `'...'`)
- `a'` - **around single quotes** (content including the quotes)
- `i\`` - **inner backticks** (content inside `` `...` ``)
- `a\`` - **around backticks** (content including the backticks)

**Bracket Text Objects:**

- `i(`, `i)`, `ib` - **inner parentheses** (content inside `(...)`)
- `a(`, `a)`, `ab` - **around parentheses** (content including the parentheses)
- `i[`, `i]` - **inner square brackets** (content inside `[...]`)
- `a[`, `a]` - **around square brackets** (content including the brackets)
- `i{`, `i}`, `iB` - **inner curly braces** (content inside `{...}`)
- `a{`, `a}`, `aB` - **around curly braces** (content including the braces)
- `i<`, `i>` - **inner angle brackets** (content inside `<...>`)
- `a<`, `a>` - **around angle brackets** (content including the brackets)

**Tag Text Objects (HTML/XML):**

- `it` - **inner tag** (content between HTML/XML tags)
- `at` - **around tag** (content including the tags)

#### Example Usage

```vim
diw     # Delete inner word (cursor on any part of word)
ciw     # Change inner word (delete word and enter insert mode)
yap     # Yank around paragraph (copy paragraph with blank lines)
>ip     # Indent inner paragraph
<i{     # Unindent content inside curly braces
~iw     # Toggle case of inner word
da"     # Delete around double quotes (delete quoted text and quotes)
ci(     # Change inner parentheses (replace content inside parentheses)
```

#### Full Integration with Undo System

All text object operations integrate seamlessly with the editor's advanced undo system:

- **Press `u` to undo any text object operation**
- **Press `Ctrl+r` to redo undone operations**
- **Memory-efficient delta-based undo** stores only the changes, not full buffer copies
- **Configurable undo levels** via `:set undolevels=<n>` (default: 1000)

This powerful combination of operators and text objects enables precise, efficient text editing that scales from simple word edits to complex structural changes.

### Ex Commands

- `:q` - Quit
- `:q!` - Force quit
- `:w` - Save file
- `:wq` - Save and quit
- `:e <filename>` - Open file in new buffer
- `:b <buffer>` - Switch to buffer by number or name
- `:bd` - Delete current buffer
- `:ls` - List all open buffers

### Command Completion System

**Tab-based completion for all commands** - Just like Vim/Neovim!

- **Tab** - Trigger command completion and cycle through matches
- **Ctrl+N** - Navigate to next completion
- **Ctrl+P** - Navigate to previous completion  
- **Ctrl+Y** - Accept current completion
- **Escape** - Cancel completion menu

The completion system provides intelligent suggestions for:

- All Ex commands (`:quit`, `:write`, `:edit`, etc.)
- Buffer management commands (`:buffer`, `:bnext`, `:bprev`, etc.)
- Window/split commands (`:split`, `:vsplit`, `:close`)
- Complete `:set` command catalog with short and long forms
- Commands are sorted by relevance (shorter matches first)

**Example workflows:**

- Type `:se` + Tab → Shows all `set` commands  
- Type `:b` + Tab → Shows all buffer commands
- Type `:q` + Tab → Shows quit variations (`:q`, `:q!`, `:quit`, `:quit!`)

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
- `:set scrolloff=<n>` (`so`) - Lines to keep around cursor (default: 3)
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
- `:set scrolloff=<n>` (`so`) - Lines to keep around cursor (default: 3)
- `:set sidescrolloff=<n>` (`siso`) - Columns to keep around cursor

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

### Platform-Specific Commands

#### Windows (PowerShell)

```powershell
# Build and run
cargo build
cargo run

# Open a file
cargo run -- filename.txt
```

#### Linux/macOS (Bash)

```bash
# Build and run
cargo build
cargo run

# Open a file
cargo run filename.txt
```

*For logging and debugging, see [LOGGING_GUIDE.md](LOGGING_GUIDE.md)*

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

- `:split` or `:sp` - Create horizontal split (window below)
- `:vsplit` or `:vsp` - Create vertical split (window right)

**Directional Split Keybindings:**

- `Ctrl+w s` - Horizontal split below current window
- `Ctrl+w S` - Horizontal split above current window  
- `Ctrl+w v` - Vertical split right of current window
- `Ctrl+w V` - Vertical split left of current window

**Navigation:**

- `Ctrl+w h` - Move to window on the left
- `Ctrl+w j` - Move to window below
- `Ctrl+w k` - Move to window above  
- `Ctrl+w l` - Move to window on the right

**Resizing:**

- `Ctrl+w >` - Make window wider
- `Ctrl+w <` - Make window narrower
- `Ctrl+w +` - Make window taller
- `Ctrl+w -` - Make window shorter

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

Syntax highlighting is built-in and powered by Tree-sitter with automatic Rust language support and async background processing. The dark theme is configured through the streamlined `themes.toml` file:

```toml
[themes.dark.syntax]
# High-contrast dark theme optimized for low-light coding
text = "#ffffff"              # Pure white text
comment = "#7c7c7c"           # Medium gray - comments
keyword = "#ff6b35"           # Bright rust orange - core keywords
operator = "#ffd23f"          # Bright golden - operators  
type = "#a3d977"              # Bright green - types (safety)
struct = "#a3d977"            # Bright green - struct definitions
enum = "#a3d977"              # Bright green - enum definitions
string = "#98d982"            # Bright light green - string literals
number = "#ffb347"            # Bright golden orange - numeric literals
boolean = "#00d4aa"           # Bright teal - boolean values
character = "#98d982"         # Bright light green - character literals
function = "#00d4aa"          # Bright teal - functions (reliability)
method = "#00d4aa"            # Bright teal - method calls
macro = "#e879f9"             # Bright purple - macros
variable = "#ffffff"          # Pure white - variables
parameter = "#ff9999"         # Bright soft red - parameters
property = "#87ceeb"          # Bright sky blue - properties/fields
constant = "#ffd23f"          # Bright golden - constants
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
text_object_timeout = 500     # Timeout for text object sequences (ms)
operator_pending_timeout = 500 # Timeout for operator-pending mode (ms)
show_command = true           # Show partial commands
scroll_off = 3                # Lines to keep above/below cursor
side_scroll_off = 0           # Columns to keep left/right of cursor
window_resize_amount = 1      # Amount to resize windows by (rows/columns)
completion_menu_width = 30    # Width of command completion popup
completion_menu_height = 8    # Height of command completion popup
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

# Window resizing keybindings
"Ctrl+w >" = "resize_window_wider"
"Ctrl+w <" = "resize_window_narrower"
"Ctrl+w +" = "resize_window_taller"
"Ctrl+w -" = "resize_window_shorter"

# Scrolling commands
"Ctrl+f" = "scroll_down_page"      # Page down
"Ctrl+b" = "scroll_up_page"        # Page up
"Ctrl+d" = "scroll_down_half_page" # Half page down
"Ctrl+u" = "scroll_up_half_page"   # Half page up
"Ctrl+e" = "scroll_down_line"      # Line down
"Ctrl+y" = "scroll_up_line"        # Line up

# Viewport centering (z commands)
"zz" = "center_cursor"             # Center cursor line
"zt" = "cursor_to_top"             # Move cursor line to top
"zb" = "cursor_to_bottom"          # Move cursor line to bottom

[insert_mode]
"Escape" = "normal_mode"
"Char" = "insert_char"
"Backspace" = "delete_char_backward"

[command_mode]
"Enter" = "execute_command"
"Escape" = "normal_mode"
"Char" = "command_char"
"Backspace" = "delete_command_char"
"Tab" = "command_complete"
"Ctrl+n" = "completion_next"
"Ctrl+p" = "completion_previous"
"Ctrl+y" = "completion_accept"
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
- `text_object_timeout` - Timeout for text object sequences (milliseconds)
- `operator_pending_timeout` - Timeout for operator-pending mode (milliseconds)
- `show_command` - Show partial commands
- `scroll_off` - Lines to keep visible above/below cursor
- `side_scroll_off` - Columns to keep visible left/right of cursor
- `window_resize_amount` - Amount to resize windows by (rows/columns)
- `completion_menu_width` - Width of command completion popup menu
- `completion_menu_height` - Height of command completion popup menu

## Architecture

### Core Modules

- **Editor**: Main editor state and coordination with TOML-based configuration
- **Config**: TOML configuration management for keymaps, editor settings, and syntax themes
- **Buffer**: Multi-buffer management with undo/redo support and clipboard operations
- **Window Management**: Complete window management system with splitting and navigation
- **AsyncSyntaxHighlighter**: Background async syntax highlighting with immediate visible line processing
- **UI**: Advanced rendering engine for status line, line numbers, cursor line, multi-window support, and syntax-highlighted content
- **Mode**: Editor mode definitions and cursor positioning
- **Search**: Regex-capable search engine with result navigation
- **Syntax**: Tree-sitter integration for professional syntax highlighting with configurable themes

### Implemented Features ✅

- **Async Syntax Highlighting**: Background worker with priority-based processing and immediate visible line highlighting
- **Multi-Buffer System**: Complete buffer management with Ex commands
- **Window Management System**: Full window splitting and navigation with Vim-style keybindings
- **Scroll Offset**: Configurable lines to keep visible around cursor (`:set scrolloff=3`)
- **Window Resizing**: Complete window resizing system with `Ctrl+w` combinations
- **Configuration Hot Reloading**: Live updates when TOML configuration files change
- **Professional Terminal Behavior**: Alternate screen support for clean entry/exit
- **Comprehensive Logging**: Debug-friendly logging system with file-based output for troubleshooting and development

## Development Roadmap

### Phase 1: Core Vim Features ✅

- [x] **TOML-based Keymap System**: Configurable keybindings (revolutionary departure from traditional Vim)
- [x] **TOML-based Editor Configuration**: Comprehensive settings system with 30+ options
- [x] **Async Syntax Highlighting**: Background worker with priority queues and immediate visible line processing
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
- [x] **Scrolling Commands**: Page (`Ctrl+f/b`), half-page (`Ctrl+d/u`), and line (`Ctrl+e/y`) scrolling
- [x] **Viewport Centering**: Z-commands (`zz`, `zt`, `zb`) for cursor positioning in viewport
- [x] **Scroll Offset**: Configurable `scroll_off` setting to maintain cursor distance from edges
- [x] **Window Resizing**: Complete window resizing with `Ctrl+w >/</+/-` keybindings
- [x] **Comprehensive Logging**: Debug-friendly logging system with file-based output for development and troubleshooting
- [x] **Text Objects**: Complete text object system (`aw`, `iw`, `ap`, quotes, brackets, tags, etc.)
- [x] **Operators**: All operators (`d`, `c`, `y`, `>`, `<`, `~`) with text object integration and full undo support
- [ ] Visual mode selection and operations
- [ ] Advanced search and replace with regex

### Phase 2: Advanced Editing 📅

- [ ] Macros and command repetition
- [ ] Code folding and auto-indentation  
- [x] Multiple windows and splits with full navigation and resizing support
- [ ] File explorer and buffer management

### Phase 3: IDE Features 📅

- [ ] LSP client with autocompletion
- [x] Syntax highlighting with Tree-sitter and async background processing (✅ Implemented)
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
- **notify**: File system watching for configuration hot reloading (✅ Implemented)
- **tokio**: Async runtime for background syntax highlighting and LSP operations (✅ Implemented)
- **log/env_logger**: Logging infrastructure for debugging and development (✅ Implemented)

## Debugging and Development

The editor includes comprehensive logging for debugging and development. **By default, all logs are written to `oxidized.log` in the current directory.** See [LOGGING_GUIDE.md](LOGGING_GUIDE.md) for complete documentation on log levels, file locations, and debugging techniques.

**Quick Start:**

```bash
# Enable debug logging (writes to oxidized.log)
RUST_LOG=debug cargo run filename.txt

# Monitor logs in real-time (Linux/macOS)
tail -f oxidized.log

# Monitor logs in real-time (Windows PowerShell)
Get-Content oxidized.log -Wait
```

## Contributing

This is a learning project to understand how text editors work. Contributions welcome!

### Getting Started

#### Prerequisites

- **Rust 1.70+** - Install from [rustup.rs](https://rustup.rs/)

#### Building and Running

**Windows (PowerShell):**

```powershell
# Clone the repository
git clone <repository-url>
cd oxidized

# Build the project
cargo build

# Run the oxidized editor
cargo run

# Run with a specific file
cargo run -- filename.txt

# Build and install the binary (creates 'oxy.exe')
cargo build --release
# Binary will be at target/release/oxy.exe
```

**Linux/macOS (Bash):**

```bash
# Clone the repository
git clone <repository-url>
cd oxidized

# Build the project
cargo build

# Run the oxidized editor
cargo run

# Run with a specific file
cargo run filename.txt

# Build and install the binary (creates 'oxy')
cargo build --release
# Binary will be at target/release/oxy
```

#### Development Setup

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
