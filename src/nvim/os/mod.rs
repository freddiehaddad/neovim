#![deny(unsafe_code)]

#[allow(unsafe_code)]
pub mod dl;
#[allow(unsafe_code)]
pub mod env;
pub mod fileio;
#[allow(unsafe_code)]
pub mod fs;
pub mod input;
#[allow(unsafe_code)]
pub mod lang;
#[allow(unsafe_code)]
pub mod os_win_console;
#[allow(unsafe_code)]
pub mod proc;
#[allow(unsafe_code)]
pub mod pty_conpty_win;
#[allow(unsafe_code)]
pub mod pty_proc_unix;
#[allow(unsafe_code)]
pub mod pty_proc_win;
#[cfg(unix)]
pub use pty_proc_unix as pty_proc;
#[cfg(windows)]
pub use pty_proc_win as pty_proc;
pub mod shell;
#[allow(unsafe_code)]
pub mod signal;
pub mod stdpaths;
#[allow(unsafe_code)]
pub mod time;
#[allow(unsafe_code)]
pub mod users;
