# Vim/Neovim Keybinding Implementation Analysis

This document analyzes the current keybinding implementation in Oxidized and identifies missing Vim/Neovim features that should be implemented for better compatibility.

## Current Implementation Status

### ✅ Well Implemented Features

#### Movement

- Basic cursor movement (`h`, `j`, `k`, `l`, arrow keys)
- Word movement (`w`, `b`, `e`)
- Line movement (`0`, `^`, `$`)
- Buffer movement (`gg`, `G`)
- Character navigation (`f{char}`, `F{char}`, `t{char}`, `T{char}`, `;`, `,`)
- Scrolling operations (`Ctrl+e`, `Ctrl+y`, `Ctrl+f`, `Ctrl+b`, `Ctrl+d`, `Ctrl+u`)
- Centering operations (`zz`, `zt`, `zb`)

#### Mode System

- Normal mode
- Insert mode
- Visual mode (character, line, block)
- Replace mode
- Command mode
- Search mode
- Operator-pending mode

#### Basic Editing

- Character deletion (`x`, `X`)
- Line deletion (`dd`)
- Line operations (`J`, `D`, `C`, `S`, `s`)
- Undo/Redo (`u`, `Ctrl+r`)
- Copy/Paste (`yy`, `yw`, `y$`, `p`, `P`)
- Repeat operations (`.`)

#### Text Objects

- Comprehensive text object support (words, sentences, paragraphs, quotes, brackets, tags)
- Works with operators (`d`, `c`, `y`)

#### Window Management

- Window splitting (`Ctrl+w s`, `Ctrl+w v`)
- Window navigation (`Ctrl+w h/j/k/l`)
- Window resizing (`Ctrl+w +/-/</>`)
- Window closing (`Ctrl+w c/q`)

#### Search

- Forward/backward search (`/`, `?`)
- Next/previous match (`n`, `N`)

---

## ❌ Missing Critical Vim Features

### High Priority (Essential for Vim compatibility)

#### 1. ✅ Character Navigation (IMPLEMENTED)

- **`f{char}`**: Find character forward on current line ✅
- **`F{char}`**: Find character backward on current line ✅
- **`t{char}`**: Till character forward (stop before character) ✅
- **`T{char}`**: Till character backward (stop before character) ✅
- **`;`**: Repeat last `f/F/t/T` motion ✅
- **`,`**: Repeat last `f/F/t/T` motion in opposite direction ✅

#### 2. ✅ Line Operations (IMPLEMENTED)

- **`J`**: Join lines (current line with next) ✅
- **`D`**: Delete to end of line ✅
- **`C`**: Change to end of line ✅
- **`S`**: Change entire line (synonym for `cc`) ✅
- **`s`**: Substitute character (delete char and enter insert mode) ✅

#### 3. ✅ Repeat Operations (IMPLEMENTED)

- **`.`**: Repeat last change (one of the most important Vim features) ✅
- This requires tracking the last editing operation ✅

#### 4. ✅ Advanced Movement (PARTIALLY IMPLEMENTED)

- **`{`**: Move to previous paragraph/block ✅
- **`}`**: Move to next paragraph/block ✅
- **`(`**: Move to previous sentence
- **`)`**: Move to next sentence
- **`%`**: Jump to matching bracket/parenthesis ✅
- **`[[`**: Move to previous section
- **`]]`**: Move to next section

#### 5. Line Numbers and Jumps

- **`{number}G`**: Go to specific line number
- **`{number}gg`**: Go to specific line number (alternative)
- **Relative line movements**: `{number}j`, `{number}k`

#### 6. Marks and Jumps

- **`m{a-z}`**: Set local mark
- **`m{A-Z}`**: Set global mark
- **`'{mark}`**: Jump to line of mark
- **`` `{mark} ``**: Jump to exact position of mark
- **`Ctrl+o`**: Jump to previous location in jump list
- **`Ctrl+i`**: Jump to next location in jump list

#### 7. Advanced Text Objects

- **Line text objects**: `_` (entire line)
- **Function text objects**: `if`, `af` (inner/around function)
- **Class text objects**: `ic`, `ac` (inner/around class)

### Medium Priority (Important for productivity)

#### 8. Register System

- **`"{register}y`**: Yank to specific register
- **`"{register}p`**: Paste from specific register
- **`""`**: Default register
- **`"0`**: Yank register
- **`"1-9`**: Delete registers
- **`"+`**: System clipboard register
- **`"*`**: Primary selection register

#### 9. Multiple Character Operations

- **`r{char}`**: Replace single character
- **`R`**: Enter replace mode (already implemented)
- **`~`**: Toggle case of character under cursor
- **`g~{motion}`**: Toggle case over motion
- **`gu{motion}`**: Lowercase over motion
- **`gU{motion}`**: Uppercase over motion

#### 10. Insert Mode Enhancements

- **`Ctrl+h`**: Delete character backward (like Backspace)
- **`Ctrl+w`**: Delete word backward (already implemented)
- **`Ctrl+u`**: Delete to beginning of line
- **`Ctrl+t`**: Indent current line
- **`Ctrl+d`**: Unindent current line
- **`Ctrl+o`**: Execute one normal mode command and return to insert

#### 11. Command Mode Enhancements

- **`:w`**: Save file (already has save_file action)
- **`:q`**: Quit (already has quit action)
- **`:wq`**: Save and quit
- **`:q!`**: Quit without saving
- **`:e {file}`**: Edit file
- **`:sp {file}`**: Split horizontally and edit file
- **`:vs {file}`**: Split vertically and edit file
- **`:{number}`**: Go to line number

#### 12. Indentation

- **`>>{motion}`**: Indent lines
- **`<<{motion}`**: Unindent lines
- **`==`**: Auto-indent current line
- **`={motion}`**: Auto-indent over motion

### Low Priority (Nice to have)

#### 13. Advanced Search

- **`*`**: Search for word under cursor forward
- **`#`**: Search for word under cursor backward
- **`g*`**: Search for partial word under cursor forward
- **`g#`**: Search for partial word under cursor backward

#### 14. Folding

- **`zf{motion}`**: Create fold
- **`zo`**: Open fold
- **`zc`**: Close fold
- **`za`**: Toggle fold
- **`zR`**: Open all folds
- **`zM`**: Close all folds

#### 15. Macros

- **`q{register}`**: Start recording macro
- **`q`**: Stop recording macro
- **`@{register}`**: Execute macro
- **`@@`**: Repeat last macro

#### 16. Tab Operations

- **`:tabnew`**: Create new tab
- **`:tabn`**: Next tab
- **`:tabp`**: Previous tab
- **`gt`**: Next tab
- **`gT`**: Previous tab

---

## Implementation Recommendations

### Phase 1: Essential Movement (High Impact, Low Complexity)

1. ✅ ~~Implement `f/F/t/T` character navigation with `;` and `,` repeat~~ **COMPLETED**
2. ✅ ~~Add `D`, `C`, `S`, `s` line operations~~ **COMPLETED**
3. ✅ ~~Implement `J` (join lines)~~ **COMPLETED**
4. ✅ ~~Add `%` bracket matching~~ **COMPLETED**

### Phase 2: Core Editing Features (High Impact, Medium Complexity)

1. ✅ ~~Implement `.` (repeat last change) - requires command recording~~ **COMPLETED**
2. ✅ ~~Add paragraph/block movement (`{`, `}`)~~ **COMPLETED**
3. Implement basic mark system (`m`, `'`, `` ` ``)
4. Add line number jumps (`{number}G`)

### Phase 3: Advanced Features (Medium Impact, High Complexity)

1. Implement register system
2. Add macro recording and playback
3. Implement folding system
4. Add advanced text objects

### Phase 4: Polish Features (Low Impact, Variable Complexity)

1. Search word under cursor (`*`, `#`)
2. Case operations (`~`, `gu`, `gU`)
3. Tab management
4. Advanced command mode features

---

## Current Architecture Assessment

The current implementation has a solid foundation:

- ✅ **Mode system**: Well implemented with proper state transitions
- ✅ **Key mapping**: Flexible TOML-based configuration
- ✅ **Action system**: Clean separation between key handling and actions
- ✅ **Text objects**: Comprehensive implementation
- ✅ **Operator system**: Proper operator-pending mode

This architecture makes it relatively straightforward to add missing features by:

1. Adding new action methods to the KeymapConfig impl
2. Adding corresponding entries to the execute_action match statement
3. Updating the keymaps.toml file with new bindings

The most complex features to implement will be:

- **Repeat system** (`.`): Requires command recording and replay
- **Register system**: Needs clipboard integration and storage
- **Macros**: Builds on the repeat system
- **Folding**: Requires text analysis and display changes

---

## Recent Implementation Updates

### August 2025 - Line Operations Implementation ✅

**Completed Features:**

- ✅ **`J` (Join Lines)**: Joins current line with next line, normalizing whitespace
- ✅ **`D` (Delete to End)**: Deletes from cursor position to end of current line
- ✅ **`C` (Change to End)**: Deletes from cursor to end of line and enters Insert mode
- ✅ **`S` (Change Line)**: Clears entire current line and enters Insert mode at beginning
- ✅ **`s` (Substitute Char)**: Deletes character under cursor and enters Insert mode

**Technical Details:**

- Added 5 new action methods to `src/keymap.rs`
- Updated `keymaps.toml` with new keybindings
- Comprehensive unit tests added (10 test functions)
- Fixed key sequence resolution issue preventing `D` and `C` from executing immediately
- All operations include proper undo support via buffer's public API

**Bug Fixes:**

- Fixed potential match detection logic that was preventing single uppercase letters (`D`, `C`) from executing immediately
- Resolved conflicts with `Down` arrow key and `Ctrl+...` sequences

**Testing:**

- 177 total tests passing (updated from 167 with bracket matching tests)
- Comprehensive coverage including edge cases (end of line, single character lines, whitespace handling)
- No regressions in existing functionality

#### Bracket Matching Implementation (% command) - August 5, 2025

**Features Added:**

- Complete bracket matching functionality for `()`, `[]`, `{}`, and `<>` pairs
- Support for nested brackets with proper stack-based counting
- Multi-line bracket matching across the entire buffer
- Bidirectional matching (opening to closing and closing to opening)
- Intelligent error handling for unmatched brackets and non-bracket characters

**Implementation Details:**

- Added `action_bracket_match` method in `src/keymap.rs`
- Added `find_matching_bracket` helper method with forward/backward search logic
- Added `%` key binding in `keymaps.toml`
- Stack-based algorithm properly handles nested bracket structures
- Comprehensive logging for debugging and user feedback

**Testing:**

- 10 comprehensive test cases covering all bracket types and edge cases
- Tests include: basic matching, nested brackets, multi-line matching, unmatched brackets, and non-bracket positions
- 189 total tests passing (added 10 new bracket matching tests)
- Full integration test validating end-to-end key handling

**Phase 1 Status: COMPLETED** ✅

All essential movement features now implemented with comprehensive testing and documentation.

#### Paragraph Movement Implementation ({ and } commands) - August 5, 2025

**Features Added:**

- Complete paragraph movement functionality with `{` (backward) and `}` (forward) navigation
- Support for navigation between paragraphs separated by empty lines
- Intelligent handling of whitespace-only lines as paragraph separators
- Cursor positioning at start of paragraph (column 0) following Vim conventions
- Edge case handling for single-line buffers and all-empty-line scenarios

**Implementation Details:**

- Added `action_paragraph_forward` and `action_paragraph_backward` methods in `src/keymap.rs`
- Added `{` and `}` key bindings in `keymaps.toml`
- Updated README.md with paragraph movement documentation in movement section
- Algorithm skips current paragraph, finds empty line separator, then locates next/previous paragraph start
- Proper boundary handling for start/end of buffer navigation

**Testing:**

- 10 comprehensive test cases covering all paragraph movement scenarios
- Tests include: basic forward/backward movement, multiple empty lines, edge cases, whitespace handling
- 189 total tests passing (added 10 new paragraph movement tests)
- Full coverage of boundary conditions and multi-paragraph navigation

**Phase 2 Status: IN PROGRESS** 🚧

Paragraph movement complete - continuing with repeat operations and mark system.

#### Repeat Operations Implementation (. command) - August 5, 2025

**Features Added:**

- Complete repeat last change functionality with `.` command
- Command recording system that tracks repeatable editing operations
- Safe replay mechanism with infinite loop prevention
- Support for all major editing operations: delete, substitute, join, put, insert modes
- Integration with existing keybinding and action execution system

**Implementation Details:**

- Added `RepeatableCommand` struct to track action, key event, and optional count
- Added `action_repeat_last_change` method in `src/keymap.rs`
- Added `record_command` and `is_repeatable_action` helper methods
- Added `.` key binding in `keymaps.toml`
- Command recording occurs automatically for all repeatable operations
- Uses `execute_action_without_recording` to prevent recursive recording during replay

**Repeatable Operations:**

- Character operations: `x`, `X`, `s`
- Line operations: `dd`, `D`, `C`, `S`, `J`
- Insert mode entries: `i`, `a`, `I`, `A`, `o`, `O`
- Put operations: `p`, `P`
- Future: case operations, indentation operations

**Testing:**

- 8 comprehensive test cases covering all repeat scenarios
- Tests include: delete char repeat, substitute char repeat, join lines repeat, put operations repeat, recording and replay validation
- 199 total tests passing (added 8 new repeat operation tests)
- Full validation of command recording, storage, and safe replay

**Bug Fixes:**

- Fixed critical bug where `dd` command would get stuck in operator-pending mode
- Added compound operator sequences (`dd`, `cc`, `yy`) to `operator_pending_mode` keymap
- Fixed `action_delete_line` to properly clear pending operator state and return to Normal mode
- Ensured text object operations (`diw`, `daw`, etc.) continue to work correctly

**Technical Implementation:**

```rust
// Command recording structure
pub struct RepeatableCommand {
    pub action: String,
    pub key: KeyEvent, 
    pub count: Option<usize>,
}

// Automatic recording during action execution
if self.is_repeatable_action(action) {
    self.record_command(action, key);
}

// Safe replay without infinite loops
self.execute_action_without_recording(editor, &last_command.action, last_command.key)?;
```

**Phase 2 Status: MAJOR PROGRESS** ✅

- ✅ Paragraph movement complete
- ✅ Repeat operations complete  
- 🚧 Next: Mark system implementation
- 🚧 Next: Line number jumps implementation
