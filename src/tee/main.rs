//! Neotee — pipe fitting (`src/tee/tee.c`).
//!
//! Reads standard input and writes what it reads to standard output and each
//! specified file.

use std::env;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::process;

const USAGE: &str = "\
Neotee: a web-scale fork of tee
Usage:
\ttee [-a] file ... file_n

\t-a\tappend to files instead of truncating

Tee reads its input, and writes to each of the specified files,
as well as to the standard output.

Shipped with Nvim 0.12+ for use with :make, :grep, :!, etc.
";

#[cfg(unix)]
fn max_open_files() -> usize {
    const SC_OPEN_MAX: std::ffi::c_int = {
        #[cfg(target_os = "linux")]
        {
            4
        }
        #[cfg(not(target_os = "linux"))]
        {
            5
        }
    };
    unsafe extern "C" {
        fn sysconf(name: std::ffi::c_int) -> std::ffi::c_long;
    }
    let v = unsafe { sysconf(SC_OPEN_MAX) };
    if v < 0 { 10 } else { v as usize }
}

#[cfg(not(unix))]
const fn max_open_files() -> usize {
    10
}

fn parse_args<I, S>(args: I) -> Result<(bool, Vec<String>), ()>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut append = false;
    let mut files = Vec::new();
    let mut parsing_opts = true;

    for raw in args {
        let arg = raw.into();
        if parsing_opts && arg.starts_with('-') {
            if arg == "-a" {
                append = true;
            } else {
                return Err(());
            }
        } else {
            parsing_opts = false;
            files.push(arg);
        }
    }

    if files.is_empty() {
        return Err(());
    }

    Ok((append, files))
}

fn tee_copy<R: Read, WOut: Write, WFile: Write>(
    mut reader: R,
    mut stdout: WOut,
    files: &mut [WFile],
) -> io::Result<()> {
    let mut buf = [0u8; 65536];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        let _ = stdout.write_all(chunk);
        for f in files.iter_mut() {
            let _ = f.write_all(chunk);
        }
        let _ = stdout.flush();
    }
    for f in files.iter_mut() {
        let _ = f.flush();
    }
    Ok(())
}

fn main() {
    let (append, filenames) = match parse_args(env::args().skip(1)) {
        Ok(v) => v,
        Err(()) => {
            let _ = io::stderr().write_all(USAGE.as_bytes());
            process::exit(2);
        }
    };

    let maxfiles = max_open_files();
    if filenames.len() + 3 > maxfiles {
        eprintln!(
            "There is a limit of max {} files.",
            maxfiles.saturating_sub(3)
        );
        process::exit(1);
    }

    let mut files: Vec<File> = Vec::with_capacity(filenames.len());
    for name in &filenames {
        match OpenOptions::new()
            .write(true)
            .create(true)
            .append(append)
            .truncate(!append)
            .open(name)
        {
            Ok(f) => files.push(f),
            Err(_) => {
                eprintln!("Can't open \"{name}\"");
                process::exit(1);
            }
        }
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let _ = tee_copy(stdin.lock(), stdout.lock(), &mut files);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_handles_append_and_rejects_invalid_flags() {
        assert_eq!(
            parse_args(["-a", "out1.txt", "out2.txt"]),
            Ok((true, vec!["out1.txt".to_string(), "out2.txt".to_string()]))
        );
        assert_eq!(
            parse_args(["out.txt"]),
            Ok((false, vec!["out.txt".to_string()]))
        );
        assert!(parse_args::<[&str; 0], &str>([]).is_err());
        assert!(parse_args(["-a"]).is_err());
        assert!(parse_args(["-x", "out.txt"]).is_err());
    }

    #[test]
    fn tee_copy_duplicates_stream_to_stdout_and_all_files() {
        let input = b"hello neotee\nsecond line\n";
        let mut out = Vec::new();
        let mut f1 = Vec::new();
        let mut f2 = Vec::new();
        {
            let mut sinks = [&mut f1, &mut f2];
            tee_copy(&input[..], &mut out, &mut sinks).unwrap();
        }
        assert_eq!(out, input);
        assert_eq!(f1, input);
        assert_eq!(f2, input);
    }
}
