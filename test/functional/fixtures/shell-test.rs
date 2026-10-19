//! Fake shell fixture for functional tests (`test/functional/fixtures/shell-test.c`).

use std::env;
use std::io::{self, BufRead, Write};
use std::process;
use std::thread;
use std::time::Duration;

fn flush_all() {
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();
}

fn flush_wait() {
    flush_all();
    thread::sleep(Duration::from_millis(10));
}

fn help() {
    println!("Fake shell");
    println!();
    println!("Usage:");
    println!("  shell-test --help");
    println!("    Prints this help to stdout.");
    println!("  shell-test");
    println!("  shell-test EXE");
    println!("    Prints \"ready $ \" to stderr.");
    println!("  shell-test -t {{prompt text}}");
    println!("    Prints \"{{prompt text}} $ \" to stderr.");
    println!("  shell-test EXE \"prog args...\"");
    println!("    Prints \"ready $ prog args...\\n\" to stderr.");
    println!("  shell-test -t {{prompt text}} EXE \"prog args...\"");
    println!("    Prints \"{{prompt text}} $ progs args...\" to stderr.");
    println!("  shell-test EXECVP \"prog\" \"arg0\" args...");
    println!("    Executes prog arg0 args... using execvp().");
    println!("  shell-test REP N {{text}}");
    println!("    Prints \"{{lnr}}: {{text}}\\n\" to stdout N times, pausing every 100 lines.");
    println!("    Example:");
    println!("      shell-test REP 97 \"foo bar\"");
    println!("      0: foo bar");
    println!("      ...");
    println!("      96: foo bar");
    println!("  shell-test REPFAST N {{text}}");
    println!("    Like REP, but print as fast as possible and then exit immediately.");
    println!("  shell-test INTERACT");
    println!("    Prints \"interact $ \" to stderr, and waits for \"exit\" input.");
    println!("  shell-test HOLD");
    println!(
        "    Prints \"holding $ \" to stderr, and waits indefinitely (without reading stdin, unlike INTERACT)."
    );
    println!("  shell-test EXIT {{code}}");
    println!("    Exits immediately with exit code \"{{code}}\".");
}

fn sscanf_int(s: &str) -> Option<i32> {
    let trimmed = s.trim_start();
    let mut end = 0;
    for (idx, ch) in trimmed.char_indices() {
        if (idx == 0 && (ch == '+' || ch == '-')) || ch.is_ascii_digit() {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    trimmed[..end].parse::<i32>().ok()
}

#[cfg(unix)]
fn drain_tty() {
    unsafe extern "C" {
        fn tcdrain(fd: std::ffi::c_int) -> std::ffi::c_int;
    }
    unsafe {
        let _ = tcdrain(1);
        let _ = tcdrain(2);
    }
}

#[cfg(not(unix))]
fn drain_tty() {}

fn run(args: &[String]) -> i32 {
    if args.len() == 2 && args[1] == "--help" {
        help();
    }

    if args.len() >= 2 {
        match args[1].as_str() {
            "-t" => {
                if args.len() < 3 {
                    eprintln!("Missing prompt text for -t option");
                    return 5;
                }
                eprint!("{} $ ", args[2]);
                if args.len() >= 5 && args[3] == "EXE" {
                    eprintln!("{}", args[4]);
                }
            }
            "EXE" => {
                if args.len() >= 3 {
                    eprintln!("ready $ {}", args[2]);
                } else {
                    eprint!("ready $ ");
                }
            }
            #[cfg(unix)]
            "EXECVP" => {
                use std::os::unix::process::CommandExt;
                if args.len() < 4 {
                    eprintln!("Not enough arguments for EXECVP");
                    return 6;
                }
                let _ = process::Command::new(&args[2])
                    .arg0(&args[3])
                    .args(&args[4..])
                    .exec();
            }
            "REP" | "REPFAST" => {
                let fast = args[1] == "REPFAST";
                if args.len() != 4 {
                    eprintln!("REP/REPFAST expects exactly 3 arguments");
                    return 4;
                }
                let Some(count) = sscanf_int(&args[2]) else {
                    eprintln!("Invalid count: {}", args[2]);
                    return 4;
                };
                for i in 0..count {
                    println!("{i}: {}", args[3]);
                    if !fast && i % 100 == 0 {
                        thread::sleep(Duration::from_millis(1));
                    }
                    flush_all();
                }
            }
            "UTF-8" => {
                let mut out = io::stdout();
                let _ = out.write_all(b"\xc3");
                flush_wait();
                let _ = out.write_all(b"\xa5\n");
                flush_wait();

                let _ = out.write_all(b"ref: \xc3\xa5\xcc\xb2\n");
                flush_wait();

                let _ = out.write_all(b"1: \xc3");
                flush_wait();
                let _ = out.write_all(b"\xa5\xcc\xb2\n");
                flush_wait();

                let _ = out.write_all(b"2: \xc3\xa5");
                flush_wait();
                let _ = out.write_all(b"\xcc\xb2\n");
                flush_wait();

                let _ = out.write_all(b"3: \xc3\xa5\xcc");
                flush_wait();
                let _ = out.write_all(b"\xb2\n");
                flush_wait();
            }
            "INTERACT" => {
                let stdin = io::stdin();
                let mut line = String::new();
                loop {
                    eprint!("interact $ ");
                    flush_all();
                    line.clear();
                    match stdin.lock().read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    let mut parts = line.split_whitespace();
                    let cmd = parts.next().unwrap_or("");
                    let arg = parts.next().and_then(sscanf_int).unwrap_or(0);
                    if cmd == "exit" {
                        return arg;
                    }
                    eprintln!("command not found: {cmd}");
                }
            }
            "HOLD" => {
                eprint!("holding $ ");
                flush_all();
                loop {
                    thread::sleep(Duration::from_secs(1));
                }
            }
            "EXIT" => {
                let mut code = 1;
                if args.len() >= 3 {
                    let Some(parsed) = sscanf_int(&args[2]) else {
                        eprintln!("Invalid exit code: {}", args[2]);
                        return 2;
                    };
                    code = parsed;
                }
                return code;
            }
            _ => {
                eprintln!("Unknown first argument: {}", args[1]);
                return 3;
            }
        }
        flush_all();
        0
    } else if args.len() == 1 {
        eprint!("ready $ ");
        flush_all();
        0
    } else {
        eprintln!("Missing first argument");
        2
    }
}

fn main() {
    #[cfg(windows)]
    {
        const CP_UTF8: u32 = 65001;
        unsafe extern "system" {
            fn SetConsoleOutputCP(wCodePageID: u32) -> i32;
        }
        unsafe {
            SetConsoleOutputCP(CP_UTF8);
        }
    }
    let args: Vec<String> = env::args().collect();
    let code = run(&args);
    flush_all();
    drain_tty();
    process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sscanf_int_parses_leading_signed_integers() {
        assert_eq!(sscanf_int("42"), Some(42));
        assert_eq!(sscanf_int(" -7abc"), Some(-7));
        assert_eq!(sscanf_int("abc"), None);
    }
}
