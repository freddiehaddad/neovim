# Oxidized Editor - Logging Guide

## Overview

The Oxidized editor includes comprehensive logging functionality to help with debugging and investigating issues. **By default, the editor logs to a file named `oxidized.log` in the current working directory.**

## Log Levels Available

- **ERROR**: Critical errors that prevent normal operation
- **WARN**: Warning conditions that don't stop execution but should be noted
- **INFO**: General information about major operations
- **DEBUG**: Detailed information useful for debugging
- **TRACE**: Very detailed trace information for fine-grained debugging

## Logging Behavior by Build Type

The logging behavior differs between debug and release builds:

- **Debug builds** (built with `cargo build`): Enable **TRACE level logging by default** for maximum debugging information, even without setting `RUST_LOG`
- **Release builds** (built with `cargo build --release`): Use standard env_logger behavior - no logging unless `RUST_LOG` environment variable is explicitly set

This means when developing, you automatically get comprehensive logging without any configuration.

**The editor logs to a file by default.** When you run the editor, it automatically creates a log file named `oxidized.log` in the current working directory. All log messages are written to this file.

### Changing to Console Logging

If you prefer to see logs in the console/terminal instead of a file, you can modify the logging configuration in `src/main.rs`:

1. Comment out the file logging code:

   ```rust
   // use env_logger::Target;
   // env_logger::Builder::from_default_env()
   //     .target(Target::Pipe(Box::new(std::fs::File::create("oxidized.log")?)))
   //     .init();
   ```

1. Uncomment the console logging:

   ```rust
   env_logger::init();
   ```

## How to Control Log Levels

### Debug Builds

Debug builds automatically enable **TRACE level logging** by default, providing comprehensive debugging information without any configuration needed. You can still override this behavior using the `RUST_LOG` environment variable if desired.

### Release Builds

Release builds use the standard env_logger behavior. You can control what level of detail is logged to the `oxidized.log` file using the `RUST_LOG` environment variable. Without setting `RUST_LOG`, release builds produce no log output.

### Windows (PowerShell)

#### Windows Debug Builds (Default: TRACE level)

Debug builds automatically provide full logging without any setup:

```powershell
# Automatically gets TRACE level logging
.\target\debug\oxy.exe filename.txt
```

To override the default behavior in debug builds:

```powershell
# Override to info level only
$env:RUST_LOG="info"; .\target\debug\oxy.exe filename.txt
```

#### Windows Release Builds (Requires RUST_LOG)

```powershell
# No logging without RUST_LOG
.\target\release\oxy.exe filename.txt

# Info level logging  
$env:RUST_LOG="info"; .\target\release\oxy.exe filename.txt

# Debug logging (shows key events, mode changes, etc.)
$env:RUST_LOG="debug"; .\target\release\oxy.exe filename.txt

# Trace logging (shows everything including individual key events)
$env:RUST_LOG="trace"; .\target\release\oxy.exe filename.txt
```

#### Module-Specific Logging (Windows)

```powershell
$env:RUST_LOG="oxidized::editor=debug"; .\target\debug\oxy.exe filename.txt
```

### Linux/macOS (Bash)

#### Linux/macOS Debug Builds (Default: TRACE level)

Debug builds automatically provide full logging without any setup:

```bash
# Automatically gets TRACE level logging
./target/debug/oxy filename.txt
```

To override the default behavior in debug builds:

```bash
# Override to info level only
RUST_LOG=info ./target/debug/oxy filename.txt
```

#### Linux/macOS Release Builds (Requires RUST_LOG)

```bash
# No logging without RUST_LOG
./target/release/oxy filename.txt

# Info level logging
RUST_LOG=info ./target/release/oxy filename.txt

# Debug logging (shows key events, mode changes, etc.)
RUST_LOG=debug ./target/release/oxy filename.txt

# Trace logging (shows everything including individual key events)
RUST_LOG=trace ./target/release/oxy filename.txt
```

#### Module-Specific Logging

```bash
RUST_LOG=oxidized::editor=debug ./target/debug/oxy filename.txt
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

When you start the editor and press : to enter command mode, the following will be written to `oxidized.log`:

```console
[INFO  oxidized::editor] Initializing Editor
[INFO  oxidized::editor] Terminal size: 193x22
[INFO  oxidized::editor] Editor initialization completed successfully
[INFO  oxidized::editor] Creating buffer 1 from file: "filename.txt"
[INFO  oxidized::editor] Starting editor main loop
[DEBUG oxidized::editor] Handling key event: KeyEvent { code: Char(':'), ... } in mode: Normal
```

**Note:** The log file `oxidized.log` is created in the directory where you run the editor.

## Debugging Common Issues

### Colon Key Not Working

Enable debug logging and press : - you should see key event handling logs in `oxidized.log`.

### File Save Issues

Enable info logging and try saving - you'll see detailed save operation logs in `oxidized.log`.

### Search Problems

Enable debug logging and perform a search - you'll see search initiation and results in `oxidized.log`.

### Buffer Management

Enable info logging when switching/closing buffers to see buffer operations in `oxidized.log`.

## Tips for Effective Debugging

1. **For development**: Use debug builds (`cargo build`) for automatic comprehensive logging
2. **For production**: Use release builds (`cargo build --release`) with selective `RUST_LOG` settings
3. Start with INFO level for general overview when using release builds
4. Use DEBUG level for input and mode issues  
5. Use TRACE level only when you need to see every key event (automatic in debug builds)
6. Use module-specific logging to focus on particular components (`RUST_LOG=oxidized::editor=debug`)
7. Check the `oxidized.log` file in your current directory for all log output
8. Use `tail -f oxidized.log` (Linux/macOS) or `Get-Content oxidized.log -Wait` (PowerShell) to monitor logs in real-time
