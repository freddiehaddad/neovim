# Neovim Editor - Logging Guide

## Overview

The editor now includes comprehensive logging functionality to help with debugging and investigating issues.

## Log Levels Available

- **ERROR**: Critical errors that prevent normal operation
- **WARN**: Warning conditions that don't stop execution but should be noted
- **INFO**: General information about major operations
- **DEBUG**: Detailed information useful for debugging
- **TRACE**: Very detailed trace information for fine-grained debugging

## How to Enable Logging

### Basic Info Logging

```bash
$env:RUST_LOG="info"; .\target\debug\neovim.exe filename.txt
```

### Debug Logging (shows key events, mode changes, etc.)

```bash
$env:RUST_LOG="debug"; .\target\debug\neovim.exe filename.txt
```

### Trace Logging (shows everything including individual key events)

```bash
$env:RUST_LOG="trace"; .\target\debug\neovim.exe filename.txt
```

### Module-Specific Logging

```bash
$env:RUST_LOG="neovim::editor=debug"; .\target\debug\neovim.exe filename.txt
```

## What Gets Logged

### Startup and Initialization

- Editor initialization status
- Terminal size detection
- Component initialization (config watcher, syntax highlighter, etc.)
- Buffer creation with file paths and IDs

### Input Handling

- Key events received (with debug/trace level)
- Mode transitions (Normal -> Command -> Search, etc.)
- Special key handling (colon key fix logging)
- Terminal resize events

### File Operations

- Buffer creation and file loading
- File save operations (success/failure)
- Buffer switching and closing
- Error conditions with file I/O

### Search Operations

- Search pattern initiation
- Search results (number of matches found)
- Search navigation (next/previous)

### Status and Error Reporting

- Status message changes
- Warning conditions
- Error recovery

## Example Log Output

When you start the editor and press : to enter command mode, you'll see:

```console
[INFO  neovim::editor] Initializing Editor
[INFO  neovim::editor] Terminal size: 193x22
[INFO  neovim::editor] Editor initialization completed successfully
[INFO  neovim::editor] Creating buffer 1 from file: "filename.txt"
[INFO  neovim::editor] Starting editor main loop
[DEBUG neovim::editor] Handling key event: KeyEvent { code: Char(':'), ... } in mode: Normal
```

## Debugging Common Issues

### Colon Key Not Working

Enable debug logging and press : - you should see key event handling logs.

### File Save Issues

Enable info logging and try saving - you'll see detailed save operation logs.

### Search Problems

Enable debug logging and perform a search - you'll see search initiation and results.

### Buffer Management

Enable info logging when switching/closing buffers to see buffer operations.

## Tips for Effective Debugging

1. Start with INFO level for general overview
2. Use DEBUG level for input and mode issues
3. Use TRACE level only when you need to see every key event
4. Use module-specific logging to focus on particular components
5. Check both stdout and stderr as some logs may go to different streams
