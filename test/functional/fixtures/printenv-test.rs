//! Functional test helper that prints an environment variable (`test/functional/fixtures/printenv-test.c`).

use std::env;
use std::io::{self, Write};
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        process::exit(1);
    }
    let key = &args[1];
    match env::var(key) {
        Ok(val) => {
            print!("{val}");
            let _ = io::stdout().flush();
        }
        Err(_) => {
            #[cfg(not(windows))]
            {
                eprint!("env var not found: {key}");
                let _ = io::stderr().flush();
            }
            process::exit(1);
        }
    }
}
