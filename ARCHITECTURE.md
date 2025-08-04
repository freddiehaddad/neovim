# Oxidized - Architecture Documentation

This document provides a comprehensive overview of the Oxidized editor's architecture, including system design, data flow, module interactions, and implementation details.

## Table of Contents

1. [System Overview](#system-overview)
2. [Module Architecture](#module-architecture)
3. [Data Flow Diagrams](#data-flow-diagrams)
4. [Configuration System](#configuration-system)
5. [Async Syntax Highlighting](#async-syntax-highlighting)
6. [Performance Optimizations](#performance-optimizations)
7. [Window Management](#window-management)
8. [Event Handling](#event-handling)
9. [Future Architecture](#future-architecture)

## System Overview

**Oxidized** is built on a modular Rust architecture designed for performance, extensibility, and maintainability. The core principle is **TOML-first configuration** combined with **async background processing** for non-blocking operations.

```text
┌──────────────────────────────────────────────────────────────────┐
│                        Oxidized Architecture                     │
├──────────────────────────────────────────────────────────────────┤
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────────────────┐  │
│  │ Terminal│  │   UI    │  │ Editor  │  │  AsyncSyntax        │  │
│  │Interface│  │Renderer │  │ Core    │  │  Highlighter        │  │
│  └─────────┘  └─────────┘  └─────────┘  └─────────────────────┘  │
│       │            │            │                 │              │
│       │            │            │                 │              │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────────────────┐  │
│  │ Keymap  │  │ Window  │  │ Buffer  │  │     Config          │  │
│  │ Handler │  │Manager  │  │Manager  │  │     System          │  │
│  └─────────┘  └─────────┘  └─────────┘  └─────────────────────┘  │
│       │            │            │                 │              │
│       │            │            │                 │              │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────────────────┐  │
│  │ Search  │  │  Mode   │  │  Theme  │  │     File            │  │
│  │ Engine  │  │ System  │  │ Manager │  │     Watcher         │  │
│  └─────────┘  └─────────┘  └─────────┘  └─────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

### Key Design Principles

1. **TOML-First Configuration**: All settings, keymaps, and themes in human-readable TOML files
2. **Async Processing**: Background workers for syntax highlighting and file operations
3. **Modular Architecture**: Clean separation of concerns with well-defined interfaces
4. **Performance Optimization**: Delta undo, caching systems, and efficient data structures
5. **Hot Reloading**: Live configuration updates without editor restart

## Module Architecture

### Core Modules Overview

```text
┌─────────────────────────────────────────────────────────────────┐
│                      Module Dependency Graph                    │
└─────────────────────────────────────────────────────────────────┘

main.rs
  └── Editor (editor.rs)
       ├── Terminal (terminal.rs)
       ├── UI (ui.rs)
       ├── WindowManager (window.rs)
       ├── KeyHandler (keymap.rs)
       ├── BufferManager (buffer.rs)
       ├── SearchEngine (search.rs)
       ├── AsyncSyntaxHighlighter (async_syntax.rs)
       ├── ConfigWatcher (config_watcher.rs)
       ├── ThemeManager (theme_watcher.rs)
       └── Config (config.rs)
            ├── EditorConfig
            ├── KeymapConfig
            └── ThemeConfig
```

### Module Responsibilities

#### 1. **Editor Core** (`editor.rs`)

- **Primary Role**: Central coordinator and state manager
- **Responsibilities**:
  - Manages all editor state (buffers, windows, mode, configuration)
  - Coordinates between all other modules
  - Handles main event loop and input processing
  - Manages buffer lifecycle and window assignments
  - Orchestrates async syntax highlighting requests

#### 2. **Async Syntax Highlighter** (`async_syntax.rs`)

- **Primary Role**: Background syntax highlighting with priority queues
- **Architecture**:

```text
┌──────────────────────────────────────────────────────────────┐
│                  AsyncSyntaxHighlighter                      │
├──────────────────────────────────────────────────────────────┤
│ Main Thread                │ Background Worker Thread        │
│                            │                                 │
│ ┌─────────────────────┐    │ ┌─────────────────────────────┐ │
│ │ Request Queue       │    │ │ Priority Queue Processor    │ │
│ │ (UnboundedSender)   │────┼─│ (High→Medium→Low→Critical)  │ │
│ └─────────────────────┘    │ └─────────────────────────────┘ │
│                            │               │                 │
│ ┌─────────────────────┐    │ ┌─────────────────────────────┐ │
│ │ Shared Cache        │◄───┼─│ Tree-sitter Highlighter     │ │
│ │ (RwLock<HashMap>)   │    │ │ (SyntaxHighlighter)         │ │
│ └─────────────────────┘    │ └─────────────────────────────┘ │
│                            │               │                 │
│ ┌─────────────────────┐    │ ┌─────────────────────────────┐ │
│ │ Immediate           │    │ │ Result Channels             │ │
│ │ Highlighting        │    │ │ (oneshot::Sender)           │ │
│ └─────────────────────┘    │ └─────────────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
```

#### 3. **Buffer Management** (`buffer.rs`)

- **Primary Role**: Text buffer operations with delta-based undo
- **Key Features**:
  - Delta-based undo system (only stores edit operations, not full buffer states)
  - Efficient text operations with minimal memory allocation
  - Cursor position tracking and validation
  - File I/O operations

#### 4. **Window Manager** (`window.rs`)

- **Primary Role**: Multi-window layout and navigation
- **Architecture**:

```text
┌─────────────────────────────────────────────────────────────┐
│                    Window Management                        │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│ ┌─────────────┐ ┌─────────────┐ ┌─────────────┐             │
│ │  Window 1   │ │  Window 2   │ │  Window 3   │             │
│ │             │ │             │ │             │             │
│ │ Buffer: 1   │ │ Buffer: 2   │ │ Buffer: 1   │             │
│ │ Viewport:   │ │ Viewport:   │ │ Viewport:   │             │
│ │ (0,0-20,80) │ │ (5,0-25,40) │ │ (10,0-30,40)│             │
│ │ Cursor:     │ │ Cursor:     │ │ Cursor:     │             │
│ │ (5,10)      │ │ (15,25)     │ │ (12,15)     │             │
│ └─────────────┘ └─────────────┘ └─────────────┘             │
│                                                             │
│ Window Layout Tree:                                         │
│ Root                                                        │
│ └── Split(Horizontal)                                       │
│     ├── Window1 (Buffer1)                                   │
│     └── Split(Vertical)                                     │
│         ├── Window2 (Buffer2)                               │
│         └── Window3 (Buffer1)                               │
└─────────────────────────────────────────────────────────────┘
```

#### 5. **Configuration System** (`config.rs`, `config_watcher.rs`)

- **Primary Role**: TOML-based configuration management with hot reloading
- **File Structure**:

```text
Configuration Files:
├── editor.toml      (Editor settings, 30+ options)
├── keymaps.toml     (Mode-specific keybindings)
└── themes.toml      (UI and syntax color themes)

Hot Reloading System:
┌─────────────────┐     ┌─────────────────┐    ┌─────────────────┐
│ File Watcher    │---> │ Change Detector │--->│ Config Reloader │
│ (notify crate)  │     │ (file modified) │    │ (apply changes) │
└─────────────────┘     └─────────────────┘    └─────────────────┘
```

## Data Flow Diagrams

### 1. Editor Startup Flow

```text
┌─────────────────────────────────────────────────────────────────┐
│                    Editor Startup Sequence                      │
└─────────────────────────────────────────────────────────────────┘

1. main.rs
   │
   ├─▶ Initialize Logging
   │   └─▶ env_logger::init()
   │
   ├─▶ Create Editor Instance
   │   ├─▶ Terminal::new() ───────────────▶ Setup crossterm, alternate screen
   │   ├─▶ EditorConfig::load() ──────────▶ Parse editor.toml
   │   ├─▶ UI::new() ────────────────────▶ Initialize renderer
   │   ├─▶ KeyHandler::new() ─────────────▶ Parse keymaps.toml
   │   ├─▶ WindowManager::new() ──────────▶ Setup window system
   │   ├─▶ ConfigWatcher::new() ──────────▶ Start file watchers
   │   ├─▶ ThemeManager::new() ───────────▶ Load themes.toml
   │   └─▶ AsyncSyntaxHighlighter::new() ─▶ Spawn background worker
   │
   ├─▶ Process Command Line Args
   │   └─▶ If file specified: create_buffer(file_path)
   │
   └─▶ Start Main Loop
       └─▶ editor.run() ──────────────────▶ Event processing loop
```

### 2. Key Event Processing Flow

```text
┌─────────────────────────────────────────────────────────────────┐
│                     Key Event Processing                        │
└─────────────────────────────────────────────────────────────────┘

Terminal Input
      │
      ▼
┌─────────────┐
│ KeyEvent    │ (crossterm::event::KeyEvent)
│ Received    │
└─────────────┘
      │
      ▼
┌─────────────┐    ┌─────────────┐    ┌─────────────┐
│ Editor      │--->│ KeyHandler  │--->│ Mode-       │
│ handle_     │    │ handle_key  │    │ specific    │
│ key_event   │    │             │    │ Processing  │
└─────────────┘    └─────────────┘    └─────────────┘
      │                  │                   │
      │                  │                   ▼
      │                  │            ┌─────────────┐
      │                  │            │ Normal Mode │
      │                  │            │ Insert Mode │
      │                  │            │ Command Mode│
      │                  │            │ Visual Mode │
      │                  │            │ Replace Mode│
      │                  │            └─────────────┘
      │                  │                   │
      ▼                  ▼                   ▼
┌─────────────┐    ┌─────────────┐    ┌─────────────┐
│ Update      │    │ Sequence    │    │ Execute     │
│ Editor      │    │ Tracking    │    │ Action      │
│ State       │    │ (dd, gg)    │    │             │
└─────────────┘    └─────────────┘    └─────────────┘
      │
      ▼
┌─────────────┐
│ Trigger     │
│ Render      │
└─────────────┘
```

### 3. Syntax Highlighting Data Flow

```text
┌─────────────────────────────────────────────────────────────────┐
│              Async Syntax Highlighting Flow                     │
└─────────────────────────────────────────────────────────────────┘

File Opening (:e filename)
      │
      ▼
┌─────────────────────────────────────────────────────────────────┐
│ request_visible_line_highlighting()                             │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│ Visible Lines (Immediate)    │    Buffer Lines (Async)          │
│                              │                                  │
│ ┌─────────────────────┐      │    ┌─────────────────────────┐   │
│ │get_immediate_       │      │    │request_highlighting     │   │
│ │highlights()         │      │    │(Priority::Medium)       │   │
│ │                     │      │    │                         │   │
│ │├─ Check Cache       │      │    │├─ Add to Worker Queue   │   │
│ │├─ Create Temp       │      │    │└─ Return oneshot::Rx    │   │
│ ││  Highlighter       │      │    │                         │   │
│ │├─ Highlight         │      │    │                         │   │
│ │├─ Update Cache      │      │    │                         │   │
│ │└─ Return Results    │      │    │                         │   │
│ └─────────────────────┘      │    └─────────────────────────┘   │
│                              │                                  │
└─────────────────────────────────────────────────────────────────┘
      │                                    │
      ▼                                    ▼
┌─────────────┐                   ┌─────────────────────┐
│ Immediate   │                   │ Background Worker   │
│ UI Update   │                   │                     │
│             │                   │ ┌─────────────────┐ │
│ All visible │                   │ │ Priority Queue  │ │
│ lines       │                   │ │ ┌─────────────┐ │ │
│ highlighted │                   │ │ │ Critical    │ │ │
│ instantly   │                   │ │ │ High        │ │ │
└─────────────┘                   │ │ │ Medium      │ │ │
                                  │ │ │ Low         │ │ │
                                  │ │ └─────────────┘ │ │
                                  │ └─────────────────┘ │
                                  │                     │
                                  │ ┌─────────────────┐ │
                                  │ │ Tree-sitter     │ │
                                  │ │ Highlighter     │ │
                                  │ └─────────────────┘ │
                                  │                     │
                                  │ ┌─────────────────┐ │
                                  │ │ Update Cache    │ │
                                  │ │ Send Results    │ │
                                  │ └─────────────────┘ │
                                  └─────────────────────┘
```

### 4. Configuration Hot Reloading Flow

```text
┌─────────────────────────────────────────────────────────────────┐
│                Configuration Hot Reloading                      │
└─────────────────────────────────────────────────────────────────┘

File System Event (editor.toml modified)
      │
      ▼
┌─────────────┐    ┌─────────────┐    ┌─────────────┐
│ File        │--->│ Config      │--->│ Detect      │
│ Watcher     │    │ Watcher     │    │ Change Type │
│ (notify)    │    │ Monitor     │    │             │
└─────────────┘    └─────────────┘    └─────────────┘
                          │                   │
                          │                   ▼
                          │            ┌─────────────┐
                          │            │ Editor:     │
                          │            │ reload_     │
                          │            │ config      │
                          │            └─────────────┘
                          │                   │
                          │                   ▼
                          │            ┌─────────────┐
                          │            │ Parse       │
                          │            │ TOML        │
                          │            │ File        │
                          │            └─────────────┘
                          │                   │
                          │                   ▼
                          │            ┌─────────────┐
                          │            │ Update      │
                          │            │ Editor      │
                          │            │ Settings    │
                          │            └─────────────┘
                          │                   │
                          │                   ▼
                          │            ┌─────────────┐
                          │            │ Update UI   │
                          │            │ Components  │
                          │            └─────────────┘
                          │                   │
                          ▼                   ▼
                  ┌─────────────┐      ┌─────────────┐
                  │ Display     │      │ Immediate   │
                  │ Status      │      │ Effect      │
                  │ Message     │      │ (No Restart)│
                  └─────────────┘      └─────────────┘
```

## Configuration System

### TOML Configuration Architecture

The editor uses a three-file TOML configuration system designed for clarity and modularity:

#### 1. **editor.toml** - Core Editor Settings

```toml
[display]
show_line_numbers = true
show_relative_numbers = false
show_cursor_line = false
color_scheme = "dark"
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

[editing]
undo_levels = 1000
persistent_undo = false
backup = false
swap_file = false
auto_save = false

[interface]
show_status_line = true
command_timeout = 1000
show_command = true
scroll_off = 3
side_scroll_off = 0
window_resize_amount = 1
```

#### 2. **keymaps.toml** - Modal Keybindings

```toml
[normal_mode]
"h" = "cursor_left"
"j" = "cursor_down"
"k" = "cursor_up"
"l" = "cursor_right"
"dd" = "delete_line"
"yy" = "yank_line"
"/" = "search_forward"
"Ctrl+w s" = "split_horizontal"
"Ctrl+w v" = "split_vertical"

[insert_mode]
"Escape" = "normal_mode"
"Char" = "insert_char"
"Backspace" = "delete_char_backward"

[command_mode]
"Enter" = "execute_command"
"Escape" = "normal_mode"
"Char" = "command_char"
```

#### 3. **themes.toml** - Streamlined Dark Theme

```toml
[theme]
current = "dark"

[themes.dark]
name = "Rust Dark"
description = "High contrast dark theme for late-night coding"

[themes.dark.ui]
background = "#0d1117"
status_bg = "#a3d977"
status_fg = "#282828"
# ... (full dark theme configuration)

[themes.dark.syntax]
text = "#ffffff"
comment = "#7c7c7c"
keyword = "#ff6b35"
# ... (full syntax color configuration)
```

### Configuration Integration

```text
┌─────────────────────────────────────────────────────────────────┐
│                   Configuration System Flow                     │
└─────────────────────────────────────────────────────────────────┘

:set number        (:set commands)
      │
      ▼
┌─────────────┐    ┌─────────────┐    ┌─────────────┐
│ Command     │--->│ Config      │--->│ Update      │
│ Parser      │    │ Updater     │    │ editor.toml │
└─────────────┘    └─────────────┘    └─────────────┘
      │                   │                   │
      │                   ▼                   ▼
      │            ┌─────────────┐    ┌─────────────┐
      │            │ Update      │    │ File        │
      │            │ Editor      │    │ Persistence │
      │            │ State       │    │             │
      │            └─────────────┘    └─────────────┘
      │                   │
      ▼                   ▼
┌─────────────┐    ┌─────────────┐
│ Immediate   │    │ UI          │
│ Effect      │    │ Update      │
│             │    │             │
└─────────────┘    └─────────────┘
```

## Async Syntax Highlighting

### Architecture Overview

The async syntax highlighting system is designed to provide immediate feedback while maintaining editor responsiveness during background processing.

#### Priority System

```text
┌─────────────────────────────────────────────────────────────────┐
│                    Priority Queue System                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│ ┌─────────────┐  Execution Order:                               │
│ │ Critical    │  1. Critical  - User actively editing line      │
│ │ Priority    │  2. High      - Currently visible lines         │
│ └─────────────┘  3. Medium    - Scroll buffer (±10 lines)       │
│ ┌─────────────┐  4. Low       - Background full file            │
│ │ High        │                                                 │
│ │ Priority    │  Benefits:                                      │
│ └─────────────┘  - Immediate visible response                   │
│ ┌─────────────┐  - Smooth scrolling experience                  │
│ │ Medium      │  - Background file processing                   │
│ │ Priority    │  - Optimal resource usage                       │
│ └─────────────┘                                                 │
│ ┌─────────────┐                                                 │
│ │ Low         │                                                 │
│ │ Priority    │                                                 │
│ └─────────────┘                                                 │
└─────────────────────────────────────────────────────────────────┘
```

#### Cache Architecture

```text
┌─────────────────────────────────────────────────────────────────┐
│                      Highlighting Cache                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│ Cache Key: HighlightCacheKey                                    │
│ ├─ content_hash: u64     (xxHash of line content)               │
│ ├─ language: String      ("rust", "python", etc.)               │
│ └─ theme: String         ("dark")                               │
│                                                                 │
│ Cache Entry: HighlightCacheEntry                                │
│ ├─ highlights: Vec<HighlightRange>                              │
│ ├─ timestamp: Instant                                           │
│ └─ access_count: u32                                            │
│                                                                 │
│ Cache Management:                                               │
│ ├─ Shared RwLock<HashMap> between main thread and worker        │
│ ├─ LRU eviction when cache exceeds 1000 entries                 │
│ ├─ Automatic invalidation on theme changes                      │
│ └─ Content-aware keys prevent stale cache hits                  │
└─────────────────────────────────────────────────────────────────┘
```

#### Dual Highlighting Approach

```text
┌─────────────────────────────────────────────────────────────────┐
│                   Dual Highlighting Strategy                    │
└─────────────────────────────────────────────────────────────────┘

File Opening / Scrolling Event
      │
      ▼
┌─────────────────────────────────────────────────────────────────┐
│ request_visible_line_highlighting()                             │
└─────────────────────────────────────────────────────────────────┘
      │
      ├─────────────────────┬─────────────────────────────────┐
      ▼                     ▼                                 ▼
┌─────────────┐     ┌─────────────┐                    ┌─────────────┐
│ Visible     │     │ Scroll      │                    │ Background  │
│ Lines       │     │ Buffer      │                    │ Buffer      │
│ (0-20)      │     │ (21-30)     │                    │ (31+)       │
└─────────────┘     └─────────────┘                    └─────────────┘
      │                     │                                 │
      ▼                     ▼                                 ▼
┌─────────────┐     ┌─────────────┐                    ┌─────────────┐
│ Immediate   │     │ Async       │                    │ Async       │
│ Highlighting│     │ Priority:   │                    │ Priority:   │
│             │     │ High        │                    │ Low         │
│ ├─ Temp     │     │             │                    │             │
│ │  Sync     │     │ ├─ Worker   │                    │ ├─ Worker   │
│ │  Highlighter    │ │  Queue    │                    │ │  Queue    │
│ ├─ Cache    │     │ └─ Cache    │                    │ └─ Cache    │
│ │  Update   │     │    Update   │                    │    Update   │
│ └─ Instant  │     │             │                    │             │
│    Result   │     │             │                    │             │
└─────────────┘     └─────────────┘                    └─────────────┘
      │                     │                                 │
      ▼                     ▼                                 ▼
┌─────────────┐     ┌─────────────┐                    ┌─────────────┐
│ Immediate   │     │ Progressive │                    │ Background  │
│ UI Update   │     │ UI Updates  │                    │ Completion  │
│ (No Wait)   │     │ (As Ready)  │                    │ (Invisible) │
└─────────────┘     └─────────────┘                    └─────────────┘
```

## Performance Optimizations

### 1. Delta-Based Undo System

Traditional editors store complete buffer states for each undo operation, leading to O(n×m) memory usage where n=buffer size and m=number of operations.

```text
┌─────────────────────────────────────────────────────────────────┐
│                   Traditional vs Delta Undo                     │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│ Traditional Undo (Memory Inefficient):                          │
│ ┌─────────────┐ ┌─────────────┐ ┌─────────────┐                 │
│ │ Full Buffer │ │ Full Buffer │ │ Full Buffer │                 │
│ │ State 1     │ │ State 2     │ │ State 3     │                 │
│ │ (1000 lines)│ │ (1000 lines)│ │ (1000 lines)│                 │
│ │ 50KB        │ │ 50KB        │ │ 50KB        │                 │
│ └─────────────┘ └─────────────┘ └─────────────┘                 │
│ Total: 150KB for 3 operations                                   │
│                                                                 │
│ Delta Undo (Memory Efficient):                                  │
│ ┌─────────────┐ ┌─────────────┐ ┌─────────────┐                 │
│ │ Insert      │ │ Delete      │ │ Replace     │                 │
│ │ Pos: 100    │ │ Pos: 200    │ │ Pos: 300    │                 │
│ │ Text: "fn"  │ │ Count: 5    │ │ Old: "old"  │                 │
│ │ 12 bytes    │ │ 8 bytes     │ │ New: "new"  │                 │
│ └─────────────┘ └─────────────┘ │ 16 bytes    │                 │
│                                 └─────────────┘                 │
│ Total: 36 bytes for 3 operations                                │
│ Memory Savings: 99.97%                                          │
└─────────────────────────────────────────────────────────────────┘
```

#### Delta Operation Types

```rust
pub enum UndoOperation {
    Insert {
        position: usize,
        text: String,
    },
    Delete {
        position: usize,
        count: usize,
        deleted_text: String,  // For redo
    },
    Replace {
        position: usize,
        old_text: String,
        new_text: String,
    },
}
```

### 2. Syntax Highlighting Cache Performance

```text
┌─────────────────────────────────────────────────────────────────┐
│                   Cache Performance Metrics                     │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│ Without Cache (Traditional):                                    │
│ ├─ Every line re-parsed on scroll                               │
│ ├─ Tree-sitter parsing: ~0.1ms per line                         │
│ ├─ 20 visible lines = 2ms per scroll                            │
│ └─ Heavy CPU usage during navigation                            │
│                                                                 │
│ With Intelligent Cache:                                         │
│ ├─ Cache hit ratio: 85-95%                                      │
│ ├─ Cache lookup: ~0.001ms per line                              │
│ ├─ 20 visible lines = 0.02ms per scroll                         │
│ └─ 100x performance improvement                                 │
│                                                                 │
│ Cache Key Strategy:                                             │
│ ├─ Content hash prevents stale hits                             │
│ ├─ Language-specific caching                                    │
│ ├─ Theme-aware invalidation                                     │
│ └─ LRU eviction for memory management                           │
└─────────────────────────────────────────────────────────────────┘
```

### 3. Async Processing Benefits

```text
┌─────────────────────────────────────────────────────────────────┐
│                  Async vs Sync Highlighting                     │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│ Synchronous Highlighting (Blocking):                            │
│ ┌─────────────────────────────────────────────────────────────┐ │
│ │ User Input │ Highlight │ UI Update │ User Input │ Highlight │ │
│ │    Event   │ Processing│           │    Event   │Processing │ │
│ │            │  (100ms)  │           │            │  (100ms)  │ │
│ └─────────────────────────────────────────────────────────────┘ │
│                    ↑                              ↑             │
│              Editor Frozen                 Editor Frozen        │
│                                                                 │
│ Asynchronous Highlighting (Non-blocking):                       │
│ ┌─────────────────────────────────────────────────────────────┐ │
│ │ User Input │ UI Update │ User Input │ UI Update │ User Input│ │
│ │    Event   │           │    Event   │           │   Event   │ │
│ │            │           │            │           │           │ │
│ └─────────────────────────────────────────────────────────────┘ │
│ ┌─────────────────────────────────────────────────────────────┐ │
│ │         Background Highlighting Worker Thread               │ │
│ │ Process │ Process │ Process │ Process │ Process │ Process   │ │
│ │ Queue   │ Queue   │ Queue   │ Queue   │ Queue   │ Queue     │ │
│ └─────────────────────────────────────────────────────────────┘ │
│                                                                 │
│ Benefits:                                                       │
│ ├─ Editor always responsive                                     │
│ ├─ Visible lines highlighted immediately                        │
│ ├─ Background processing for non-visible content                │
│ └─ Optimal CPU utilization                                      │
└─────────────────────────────────────────────────────────────────┘
```

## Window Management

### Window Layout System

The editor implements a flexible window management system supporting arbitrary splits and layouts:

```text
┌─────────────────────────────────────────────────────────────────┐
│                     Window Layout Tree                          │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│ Root (Terminal Size: 80x24)                                     │
│ │                                                               │
│ ├── Split(Horizontal, pos=12)                                   │
│ │   │                                                           │
│ │   ├── Window1 (0,0 -> 80,12)                                  │
│ │   │   ├── buffer_id: Some(1)                                  │
│ │   │   ├── viewport_top: 0                                     │
│ │   │   └── cursor: (5, 10)                                     │
│ │   │                                                           │
│ │   └── Split(Vertical, pos=40)                                 │
│ │       │                                                       │
│ │       ├── Window2 (0,12 -> 40,24)                             │
│ │       │   ├── buffer_id: Some(2)                              │
│ │       │   ├── viewport_top: 0                                 │
│ │       │   └── cursor: (3, 15)                                 │
│ │       │                                                       │
│ │       └── Window3 (40,12 -> 80,24)                            │
│ │           ├── buffer_id: Some(1)                              │
│ │           ├── viewport_top: 10                                │
│ │           └── cursor: (12, 5)                                 │
│ │                                                               │
│ └── Active Window: Window1                                      │
└─────────────────────────────────────────────────────────────────┘
```

### Window Operations

```text
Window Creation:
├── :split / Ctrl+w s    (Horizontal split below)
├── :vsplit / Ctrl+w v   (Vertical split right)
├── Ctrl+w S             (Horizontal split above)
└── Ctrl+w V             (Vertical split left)

Window Navigation:
├── Ctrl+w h/j/k/l       (Move to adjacent window)
├── Ctrl+w w             (Cycle through windows)
└── Mouse click          (Direct window selection - planned)

Window Resizing:
├── Ctrl+w >/<           (Wider/narrower)
├── Ctrl+w +/-           (Taller/shorter)
├── :resize [N]          (Set height)
└── :vertical resize [N] (Set width)

Window Management:
├── Ctrl+w c             (Close current window)
├── Ctrl+w o             (Close all other windows)
└── :only                (Close all other windows)
```

## Event Handling

### Event Processing Architecture

```text
┌─────────────────────────────────────────────────────────────────┐
│                       Event Flow Diagram                        │
└─────────────────────────────────────────────────────────────────┘

Terminal Events
      │
      ▼
┌─────────────┐
│ crossterm   │
│ Event Poll  │
│ (1ms timeout)
└─────────────┘
      │
      ├─────────────┬─────────────┬────────────────────┐
      ▼             ▼             ▼                    ▼
┌─────────────┐ ┌─────────────┐ ┌─────────────┐ ┌─────────────┐
│ Key Event   │ │ Mouse Event │ │ Resize Event│ │ Other Events│
└─────────────┘ └─────────────┘ └─────────────┘ └─────────────┘
      │             │             │                    │
      ▼             ▼             ▼                    ▼
┌─────────────┐ ┌─────────────┐ ┌─────────────┐ ┌─────────────┐
│ Key Handler │ │ Mouse       │ │ Terminal    │ │ Future      │
│             │ │ Handler     │ │ Resize      │ │ Extensions  │
│             │ │ (planned)   │ │ Handler     │ │             │
└─────────────┘ └─────────────┘ └─────────────┘ └─────────────┘
      │
      ▼
┌─────────────┐
│ Mode-based  │
│ Processing  │
└─────────────┘
      │
      ├────────────────┬───────────────┬───────────────┬───────────────┐
      ▼                ▼               ▼               ▼               ▼
┌─────────────┐ ┌─────────────┐ ┌─────────────┐ ┌─────────────┐ ┌─────────────┐
│ Normal Mode │ │ Insert Mode │ │ Command Mode│ │ Visual Mode │ │ Replace Mode│
│ Navigation  │ │ Text Input  │ │ Ex Commands │ │ Selection   │ │ Overwrite   │
│ Commands    │ │ Char Insert │ │ Execution   │ │ Operations  │ │ Operations  │
└─────────────┘ └─────────────┘ └─────────────┘ └─────────────┘ └─────────────┘
      │                │               │               │               │
      └────────────────┴───────────────┴───────────────┴───────────────┘
                                       │
                                       ▼
                                ┌─────────────┐
                                │ Editor      │
                                │ State       │
                                │ Update      │
                                └─────────────┘
                                       │
                                       ▼
                                ┌─────────────┐
                                │ Trigger     │
                                │ Render      │
                                └─────────────┘
```

### Key Sequence Processing

```text
┌─────────────────────────────────────────────────────────────────┐
│                    Key Sequence Handling                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│ Example: "dd" (delete line)                                     │
│                                                                 │
│ Key 'd' pressed                                                 │
│      │                                                          │
│      ▼                                                          │
│ ┌─────────────┐    ┌─────────────┐    ┌─────────────┐           │
│ │ Key Handler │--->│ Sequence    │--->│ Partial     │           │
│ │             │    │ Buffer      │    │ Match       │           │
│ │             │    │ "d"         │    │ Found       │           │
│ └─────────────┘    └─────────────┘    └─────────────┘           │
│                           │                   │                 │
│                           │                   ▼                 │
│                           │            ┌─────────────┐          │
│                           │            │ Start       │          │
│                           │            │ Timer       │          │
│                           │            │ (1000ms)    │          │
│                           │            └─────────────┘          │
│                           │                                     │
│ Key 'd' pressed again     │                                     │
│      │                    │                                     │
│      ▼                    ▼                                     │
│ ┌─────────────┐    ┌─────────────┐    ┌─────────────┐           │
│ │ Key Handler │--->│ Sequence    │--->│ Complete    │           │
│ │             │    │ Buffer      │    │ Match       │           │
│ │             │    │ "dd"        │    │ Found       │           │
│ └─────────────┘    └─────────────┘    └─────────────┘           │
│                                              │                  │
│                                              ▼                  │
│                                       ┌─────────────┐           │
│                                       │ Execute     │           │
│                                       │ delete_line │           │
│                                       │ Action      │           │
│                                       └─────────────┘           │
└─────────────────────────────────────────────────────────────────┘
```

## Future Architecture

### Planned Enhancements

#### 1. LSP Integration Architecture

```text
┌─────────────────────────────────────────────────────────────────┐
│                      LSP Client Architecture                    │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│ Editor Core                                                     │
│      │                                                          │
│      ▼                                                          │
│ ┌─────────────┐    ┌─────────────┐    ┌─────────────┐           │
│ │ LSP Client  │<-->│ JSON-RPC    │<-->│ Language    │           │
│ │ Manager     │    │ Transport   │    │ Server      │           │
│ │             │    │             │    │ (rust-analyzer)         │
│ └─────────────┘    └─────────────┘    └─────────────┘           │
│      │                                                          │
│      ▼                                                          │
│ ┌─────────────┐                                                 │
│ │ Diagnostics │                                                 │
│ │ Manager     │                                                 │
│ │             │                                                 │
│ │ ├─ Errors   │                                                 │
│ │ ├─ Warnings │                                                 │
│ │ └─ Hints    │                                                 │
│ └─────────────┘                                                 │
│                                                                 │
│ Features:                                                       │
│ ├─ Auto-completion                                              │
│ ├─ Go-to definition                                             │
│ ├─ Hover documentation                                          │
│ ├─ Real-time diagnostics                                        │
│ └─ Code actions                                                 │
└─────────────────────────────────────────────────────────────────┘
```

#### 2. Plugin System Architecture

```text
┌─────────────────────────────────────────────────────────────────┐
│                       Plugin System                             │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│ Editor Core                                                     │
│      │                                                          │
│      ▼                                                          │
│ ┌─────────────┐    ┌─────────────┐    ┌─────────────┐           │
│ │ Plugin      │--->│ Lua Runtime │--->│ Plugin API  │           │
│ │ Manager     │    │ (mlua)      │    │ Bindings    │           │
│ │             │    │             │    │             │           │
│ └─────────────┘    └─────────────┘    └─────────────┘           │
│      │                                       │                  │
│      ▼                                       ▼                  │
│ ┌─────────────┐                       ┌─────────────┐           │
│ │ Plugin      │                       │ Editor      │           │
│ │ Discovery   │                       │ Commands    │           │
│ │             │                       │ & Events    │           │
│ │ ├─ ~/.nvim/ │                       │             │           │
│ │ │  plugins/ │                       │ ├─ buffer   │           │
│ │ └─ manifest │                       │ ├─ window   │           │
│ └─────────────┘                       │ ├─ keymap   │           │
│                                       │ └─ ui       │           │
│                                       └─────────────┘           │
└─────────────────────────────────────────────────────────────────┘
```

#### 3. Multi-Language Support

```text
┌─────────────────────────────────────────────────────────────────┐
│                  Language Support Architecture                  │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│ File Detection                                                  │
│      │                                                          │
│      ▼                                                          │
│ ┌─────────────┐    ┌─────────────┐    ┌─────────────┐           │
│ │ File        │--->│ Language    │--->│ Tree-sitter │           │
│ │ Extension   │    │ Registry    │    │ Grammar     │           │
│ │ Detection   │    │             │    │ Loader      │           │
│ └─────────────┘    └─────────────┘    └─────────────┘           │
│                           │                   │                 │
│                           ▼                   ▼                 │
│                    ┌─────────────┐    ┌─────────────┐           │
│                    │ LSP Server  │    │ Syntax      │           │
│                    │ Selection   │    │ Theme       │           │
│                    │             │    │ Mapping     │           │
│                    └─────────────┘    └─────────────┘           │
│                                                                 │
│ Supported Languages (Planned):                                  │
│ ├─ Rust (Done)                                                  │
│ ├─ Python                                                       │
│ ├─ JavaScript/TypeScript                                        │
│ ├─ Go                                                           │
│ ├─ C/C++                                                        │
│ └─ Extensible via Tree-sitter grammars                          │
└─────────────────────────────────────────────────────────────────┘
```

---

## Conclusion

This architecture documentation provides a comprehensive overview of the Oxidized editor's design and implementation. The system is built around the core principles of:

1. **TOML-First Configuration** - Human-readable, persistent, hot-reloadable settings
2. **Async Processing** - Non-blocking operations for responsive user experience  
3. **Performance Optimization** - Delta undo, intelligent caching, efficient data structures
4. **Modular Design** - Clean separation of concerns with well-defined interfaces
5. **Extensibility** - Designed for future enhancement with LSP, plugins, and multi-language support

The current implementation provides a solid foundation for a modern text editor while maintaining familiar Vim-style interactions and adding revolutionary configuration management through TOML files.

For implementation details of specific modules, refer to the source code in the `src/` directory.
